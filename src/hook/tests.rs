#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ActiveRec;

    #[test]
    fn cursor_response_maps_tool_output_to_tool_response() {
        // Claude Code / Codex send `tool_response`; it wins when present.
        assert_eq!(
            cursor_response(
                serde_json::json!({ "exit_code": 0 }),
                Some(serde_json::json!("ignored"))
            ),
            serde_json::json!({ "exit_code": 0 })
        );
        // Cursor sends `tool_output` as a JSON-stringified string → parsed back to JSON.
        assert_eq!(
            cursor_response(
                serde_json::Value::Null,
                Some(serde_json::json!("{\"ok\":true}"))
            ),
            serde_json::json!({ "ok": true })
        );
        // A non-JSON tool_output string is kept verbatim rather than dropped.
        assert_eq!(
            cursor_response(serde_json::Value::Null, Some(serde_json::json!("plain"))),
            serde_json::json!("plain")
        );
        // Nothing present → null.
        assert_eq!(
            cursor_response(serde_json::Value::Null, None),
            serde_json::Value::Null
        );
    }

    #[test]
    fn strips_a_base64_screenshot_but_keeps_the_action() {
        let big = "iVBORw0KGgoAAAANSUhEUg".repeat(100); // long base64-looking blob
        let mut response = serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": big }
        });
        let mut input = serde_json::json!({ "action": "screenshot" });
        strip_screenshots(&mut response);
        strip_screenshots(&mut input);
        // The pixels are gone, replaced by a marker.
        let data = response["source"]["data"].as_str().unwrap();
        assert!(data.contains("stripped screenshot"), "{data}");
        assert!(!data.contains("iVBORw0KGgo"));
        // The action is untouched.
        assert_eq!(input["action"], "screenshot");
    }

    #[test]
    fn decode_base64_matches_known_values() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        // padding optional, embedded whitespace skipped
        assert_eq!(decode_base64("aGVs\nbG8").unwrap(), b"hello");
        // an out-of-alphabet byte yields nothing rather than garbage
        assert!(decode_base64("abc!def").is_none());
    }

    #[test]
    fn keep_frames_writes_a_png_and_ignores_action_fields() {
        let b64 = "iVBORw0KGgoAAAANSUhEUg".repeat(4); // ≥64 base64-ish chars
        let response = serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": b64 }
        });
        // Action fields are not images: they must produce no frame.
        let input = serde_json::json!({ "action": "screenshot", "coordinate": [10, 20] });

        let dir = std::env::temp_dir().join(format!("galdr-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_image_blobs(&dir, 7, &input, &response);
        assert_eq!(
            written, 1,
            "exactly the one screenshot blob becomes a frame"
        );

        let png = dir.join("0007.png");
        assert!(png.exists(), "frame written at the seq-named path");
        assert_eq!(
            std::fs::read(&png).unwrap(),
            decode_base64(&b64).unwrap(),
            "the frame holds the decoded pixels"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strip_leaves_short_data_and_prose_alone() {
        // A short `data` value (e.g. a small payload) and ordinary prose are kept.
        let mut v = serde_json::json!({
            "data": "ok",
            "command": "git status",
            "note": "a normal sentence with spaces, not base64"
        });
        let before = v.clone();
        strip_screenshots(&mut v);
        assert_eq!(v, before);
    }

    #[test]
    fn strip_only_acts_with_image_context_not_generic_base64() {
        let blob = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVowMTIzNDU2Nzg5".repeat(50);
        // No image context: a large base64-looking value under `stdout` is PRESERVED
        // (stripping it would be irreversible data loss for an arbitrary tool).
        let mut generic = serde_json::json!({ "stdout": blob });
        let before = generic.clone();
        strip_screenshots(&mut generic);
        assert_eq!(generic, before, "generic base64 must not be stripped");

        // A data: image URI is stripped regardless of context.
        let mut uri = serde_json::json!({ "url": format!("data:image/png;base64,{blob}") });
        strip_screenshots(&mut uri);
        assert!(uri["url"].as_str().unwrap().contains("stripped screenshot"));

        // An image-ish key with base64url payload is stripped.
        let mut keyed = serde_json::json!({ "image": blob.replace('+', "-") });
        strip_screenshots(&mut keyed);
        assert!(
            keyed["image"]
                .as_str()
                .unwrap()
                .contains("stripped screenshot")
        );
    }

    fn mk(rec_id: &str, origin: Option<&str>, bound: Option<&str>) -> ActiveRec {
        ActiveRec {
            rec_id: rec_id.into(),
            name: "t".into(),
            started_at: "ts".into(),
            transcript_path: None,
            origin_cwd: origin.map(String::from),
            bound_session: bound.map(String::from),
        }
    }

    /// Asserts the route records to `rec_id` and binds (or not) as expected.
    fn assert_records(route: Route, rec_id: &str, bind: Option<&str>) {
        match route {
            Route::Record {
                rec_id: got,
                bind: got_bind,
            } => {
                assert_eq!(got, rec_id, "wrong recording");
                assert_eq!(got_bind.as_deref(), bind, "wrong binding");
            }
            Route::Skip => panic!("expected a Record to {rec_id}, got Skip"),
        }
    }

    #[test]
    fn binds_to_the_first_session_under_origin() {
        let actives = [mk("01A", Some("/proj/galdr"), None)];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/proj/galdr/sub"), |_| false),
            "01A",
            Some("sessA"),
        );
    }

    #[test]
    fn foreign_session_in_another_dir_is_skipped_before_binding() {
        let actives = [mk("01A", Some("/proj/galdr"), None)];
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/proj/eldr"), |_| false),
            Route::Skip
        ));
    }

    #[test]
    fn once_bound_only_that_session_records() {
        let actives = [mk("01A", Some("/proj/galdr"), Some("sessA"))];
        // The bound session records without re-binding.
        assert_records(
            route_event(&actives, Some("sessA"), Some("/anywhere"), |_| false),
            "01A",
            None,
        );
        // Another session cannot claim the already-bound recording.
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/proj/galdr"), |_| false),
            Route::Skip
        ));
    }

    #[test]
    fn sessionless_events_record_to_the_newest_active() {
        // Harnesses that omit session_id, and the single-recording case, keep working.
        let unbound = [mk("01A", Some("/proj/galdr"), None)];
        assert_records(
            route_event(&unbound, None, Some("/tmp"), |_| false),
            "01A",
            None,
        );
        let bound = [mk("01A", Some("/proj/galdr"), Some("sessA"))];
        assert_records(route_event(&bound, None, None, |_| false), "01A", None);
        // With several active, a session-less event goes to the newest (01C).
        let many = [
            mk("01C", None, None),
            mk("01B", None, Some("sessB")),
            mk("01A", None, None),
        ];
        assert_records(route_event(&many, None, None, |_| false), "01C", None);
    }

    #[test]
    fn no_origin_binds_to_any_first_session() {
        let actives = [mk("01A", None, None)];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/anywhere"), |_| false),
            "01A",
            Some("sessA"),
        );
    }

    #[test]
    fn routes_to_the_bound_recording_among_several() {
        // Two recordings, each bound to a different session: each session's events go
        // only to its own recording — no cross-contamination.
        let actives = [
            mk("01B", Some("/projB"), Some("sessB")),
            mk("01A", Some("/projA"), Some("sessA")),
        ];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/projA"), |_| false),
            "01A",
            None,
        );
        assert_records(
            route_event(&actives, Some("sessB"), Some("/projB"), |_| false),
            "01B",
            None,
        );
        // A third, unknown session with no unbound recording to claim is dropped.
        assert!(matches!(
            route_event(&actives, Some("sessC"), Some("/projC"), |_| false),
            Route::Skip
        ));
    }

    #[test]
    fn binds_the_most_recent_unbound_eligible_recording() {
        // Newest-first. Two unbound recordings the session's cwd matches: the most
        // recent (01C) is claimed; a same-cwd session then finds only 01B left.
        let actives = [
            mk("01C", Some("/proj"), None),
            mk("01B", Some("/proj"), None),
            mk("01A", Some("/other"), Some("old")),
        ];
        assert_records(
            route_event(&actives, Some("sessNew"), Some("/proj/x"), |_| false),
            "01C",
            Some("sessNew"),
        );
    }

    #[test]
    fn unbound_recording_in_another_dir_is_not_claimed() {
        // The only unbound recording lives under a different origin: an event from
        // elsewhere must not bind it (no stealing across directories).
        let actives = [mk("01A", Some("/projA"), None)];
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/projB"), |_| false),
            Route::Skip
        ));
    }

    #[test]
    fn stale_recordings_neither_take_sessionless_events_nor_bind_new_sessions() {
        let actives = [mk("01C", None, None), mk("01B", None, None)];
        let newest_is_stale = |a: &ActiveRec| a.rec_id == "01C";
        // A session-less event skips the stale newest and lands on the next fresh one.
        assert_records(
            route_event(&actives, None, None, newest_is_stale),
            "01B",
            None,
        );
        // A new session cannot bind a stale recording either: the fresh one claims it.
        assert_records(
            route_event(&actives, Some("s1"), None, newest_is_stale),
            "01B",
            Some("s1"),
        );
        // Everything stale → the event is dropped, not appended to a zombie.
        let all_stale = [mk("01C", None, None)];
        assert!(matches!(
            route_event(&all_stale, None, None, |_| true),
            Route::Skip
        ));
        // But the bound owner still records: its own activity un-stales the span.
        let bound = [mk("01A", None, Some("sessA"))];
        assert_records(
            route_event(&bound, Some("sessA"), None, |_| true),
            "01A",
            None,
        );
    }

    #[test]
    fn path_within_respects_component_boundaries() {
        assert!(path_within("/a/b", "/a/b"));
        assert!(path_within("/a/b/c", "/a/b"));
        assert!(!path_within("/a/bc", "/a/b"));
        assert!(!path_within("/x", "/a/b"));
    }

    #[test]
    fn galdr_control_commands_are_not_recorded() {
        let ctl =
            |cmd: &str| is_galdr_control_command("Bash", &serde_json::json!({ "command": cmd }));
        assert!(ctl("galdr rec start my-task"));
        assert!(ctl("galdr rec stop"));
        assert!(ctl("galdr rec status"));
        // Tolerates a leading cd and an absolute path to the binary.
        assert!(ctl("cd /repo && galdr rec start x"));
        assert!(ctl("/Users/me/.cargo/bin/galdr rec stop"));
        assert!(ctl("galdr rec start x >/dev/null"));
        // Real task commands — including other galdr subcommands — are recorded.
        assert!(!ctl("galdr distill 01ABC"));
        assert!(!ctl("cargo test"));
        // A control phrase quoted inside another command stays recorded.
        assert!(!ctl("git commit -m 'galdr rec start'"));
        // Real work bundled into the same call is kept, not dropped with the control cmd.
        assert!(!ctl("galdr rec start x && cargo build"));
        assert!(!ctl(
            "galdr rec start x\nVER=$(grep version Cargo.toml)\necho $VER"
        ));
        // Only Bash is inspected; an unrelated tool with such input is not a control cmd.
        assert!(!is_galdr_control_command(
            "Read",
            &serde_json::json!({ "command": "galdr rec start x" })
        ));
    }
}
