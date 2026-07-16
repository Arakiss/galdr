#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_launch_args_avoid_keychain_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let session = BrowserObserveSession {
            rec_id: "rec".to_string(),
            name: "demo".to_string(),
            url: "https://example.test".to_string(),
            started_at: "2026-06-30T00:00:00Z".to_string(),
            port: 34343,
            devtools_port: 34344,
            session_dir: tmp.path().to_path_buf(),
            sensor_dir: tmp.path().join("sensor"),
            profile_dir: tmp.path().join("profile"),
            events_file: tmp.path().join("events.ndjson"),
            log_file: tmp.path().join("server.log"),
            cwd: None,
            server_pid: None,
            browser_pid: None,
            headless: false,
        };

        let args = browser_args(&session);

        assert!(args.iter().any(|arg| arg == "--use-mock-keychain"));
        assert!(args.iter().any(|arg| arg == "--password-store=basic"));
        assert!(args.iter().any(|arg| arg == "--disable-sync"));
        assert!(
            args.iter()
                .any(|arg| arg == "--remote-debugging-address=127.0.0.1")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "--remote-debugging-port=34344")
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--load-extension")));
        assert!(
            args.iter().any(|arg| {
                arg == &format!("--user-data-dir={}", session.profile_dir.display())
            })
        );
    }

    #[test]
    fn browser_executable_resolves_macos_app_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Test Browser.app");
        let executable = app.join("Contents/MacOS/Test Browser");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "").unwrap();

        assert_eq!(browser_executable(&app), executable);
    }
}
