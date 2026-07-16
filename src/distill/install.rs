/// Distills recording `id` into an installed skill.
///
/// Default: render the faithful skill from the span and install it as an unauthored
/// *draft*, then hand the agent a brief to author it (the intelligence galdr can't
/// supply) and install with `from`. With `fast`, install that mechanical render as a
/// final skill in one step. With `from`, install the `SKILL.md` the agent prepared. In
/// every case galdr is the only writer of the skills directory.
pub fn distill(
    id: &str,
    from: Option<&Path>,
    fast: bool,
    strict: bool,
    name: Option<&str>,
) -> Result<()> {
    let recording = load_recording(id)?;
    let skill_name = skill_name_for(name, &recording);
    let skill_dir = paths::skill_dir(&skill_name)?;

    if let Some(src) = from {
        let content = std::fs::read_to_string(src)
            .with_context(|| format!("could not read the distillation at {}", src.display()))?;
        validate_skill_md(&content)?;
        return install_from(id, &recording, &content, name, strict);
    }

    // `--fast` accepts the mechanical render as final; the default hands the agent a
    // faithful draft to author, because a replay of the tool calls is not yet a skill.
    if fast {
        return write_complete(id, &skill_name, &skill_dir, &recording, strict);
    }
    write_draft(id, &skill_name, &skill_dir, &recording, strict)
}

/// Installs a skill the agent authored (`--from`). An explicit `--name` always sets the
/// install name. Otherwise, when the authored frontmatter carries a valid kebab-case
/// `name:` that differs from the mechanical recording slug, galdr installs under *that*
/// name — so the directory, the skill's own `name:`, and the catalog key all agree,
/// instead of installing under the recording slug and splitting the skill's identity
/// (the frontmatter name that `galdr validate` and the reindexed catalog key on). A
/// leftover slug directory from a prior draft of the same recording is retired so no
/// duplicate remains; a frontmatter name already held by an unrelated skill is refused
/// without touching anything.
fn install_from(
    id: &str,
    recording: &record::Recording,
    content: &str,
    explicit_name: Option<&str>,
    strict: bool,
) -> Result<()> {
    let ctx = validate::ValidationCtx::new(false, strict);

    // An explicit --name wins outright — no frontmatter inference.
    if let Some(name) = explicit_name {
        let install_name = slugify(name);
        let install_dir = paths::skill_dir(&install_name)?;
        install_skill(&install_name, &install_dir, content, id, &ctx)?;
        return Ok(());
    }

    let recording_slug = skill_name_for(None, recording); // galdr-<slug>
    let Some(install_name) = honored_frontmatter_name(content, &recording_slug) else {
        // Nothing to honor: install under the recording slug, as before.
        let install_dir = paths::skill_dir(&recording_slug)?;
        install_skill(&recording_slug, &install_dir, content, id, &ctx)?;
        return Ok(());
    };

    // The frontmatter renames the skill. Refuse to overwrite an UNRELATED skill that
    // already holds that name, before touching anything.
    let install_dir = paths::skill_dir(&install_name)?;
    guard_name_collision(&install_name, &install_dir, id)?;

    install_skill(&install_name, &install_dir, content, id, &ctx)?;

    // A prior draft of THIS recording under the mechanical slug would leave a second
    // copy behind; retire it so the rename is clean.
    migrate_stale_slug_dir(&recording_slug, id);

    println!("installed as {install_name} (renamed from recording slug {recording_slug})");
    Ok(())
}

/// The frontmatter `name:` to honor for a `--from` install, or `None`. Only a name that
/// is already kebab-case (the same normalization `slugify` applies to a chosen name) and
/// actually differs from the recording slug is honored — so the installed directory can
/// equal the name verbatim and identity never splits. A name that would change under
/// slugify is left alone (installing under a slugified variant would split identity just
/// the same), as is one that already equals the slug.
fn honored_frontmatter_name(content: &str, recording_slug: &str) -> Option<String> {
    let name = crate::catalog::parse_frontmatter_name(content)?;
    if name != slugify(&name) {
        return None;
    }
    (name != recording_slug).then_some(name)
}

/// Refuses a frontmatter rename that would land on an UNRELATED existing skill. A
/// directory is "related" (safe to replace) only when its `SKILL.md` traces back to the
/// same recording — a prior draft or version of the very skill being installed. Anything
/// else is someone else's skill: bail without touching it.
fn guard_name_collision(install_name: &str, install_dir: &Path, rec_id: &str) -> Result<()> {
    let Ok(existing) = std::fs::read_to_string(install_dir.join("SKILL.md")) else {
        return Ok(()); // nothing installed there — safe to write
    };
    if crate::catalog::parse_rec_id(&existing).as_deref() == Some(rec_id) {
        return Ok(()); // our own prior draft/version — replacing it is fine
    }
    bail!(
        "cannot install as '{install_name}': a different skill already exists at {} (from another recording). \
         Rename the frontmatter `name:`, or retire that skill first with `galdr rm {install_name}`.",
        tilde(install_dir)
    );
}

/// Retires a leftover mechanical-slug directory once a `--from` install landed under a
/// frontmatter name — but only when that directory is a prior draft/version of THIS
/// recording (its provenance matches), never an unrelated skill that merely shares the
/// slug. Best-effort: the rename already succeeded, so a failed cleanup only leaves a
/// harmless duplicate and must not fail the install.
fn migrate_stale_slug_dir(recording_slug: &str, rec_id: &str) {
    let Ok(slug_dir) = paths::skill_dir(recording_slug) else {
        return;
    };
    if !slug_dir.is_dir() {
        return;
    }
    let Ok(existing) = std::fs::read_to_string(slug_dir.join("SKILL.md")) else {
        return;
    };
    if crate::catalog::parse_rec_id(&existing).as_deref() != Some(rec_id) {
        return; // an unrelated skill happens to sit under the slug — leave it
    }
    match crate::remove::retire_skill_dir(recording_slug) {
        Ok(outcome) => println!(
            "  retired the stale {recording_slug} draft → {}",
            tilde(&outcome.moved_to)
        ),
        Err(err) => {
            eprintln!("warning: could not retire the stale {recording_slug} draft: {err:#}")
        }
    }
}

/// The `--fast` (and `--auto` fallback) path: render the faithful skill from the span
/// and install it as final. No agent pass — it is the mechanical floor, usable as-is.
/// A replay of the tool calls, honest about being exactly that; the default flow hands
/// the agent the same render as a draft to elevate.
fn write_complete(
    id: &str,
    skill_name: &str,
    skill_dir: &Path,
    recording: &record::Recording,
    strict: bool,
) -> Result<()> {
    let span_path = paths::span_file(id)?;
    let events = span::read_span(&span_path)
        .with_context(|| format!("could not read span {}", span_path.display()))?;

    let content = render_complete_skill(skill_name, recording, &events);
    // The deterministic render must satisfy galdr's own validator — a guarantee that
    // the default path never produces something it would reject from `--from`.
    validate_skill_md(&content)
        .context("internal: the complete distiller produced an invalid skill")?;
    let ctx = validate::ValidationCtx::new(false, strict);
    install_skill(skill_name, skill_dir, &content, id, &ctx)?;

    println!(
        "Distilled {} step(s) into a complete skill (mechanical render, installed as final).",
        meaningful_steps(&events).len()
    );
    println!("For a sharper skill, drop `--fast` next time and author it from the span.");
    Ok(())
}

/// The default flow: write the faithful render as an *unauthored draft* (status draft,
/// not linked into harnesses — a mechanical skill should not reach an agent until it is
/// authored) and print an authoring brief. The agent reads the span, supplies the
/// judgment galdr can't, and installs the elevated skill with `--from`.
fn write_draft(
    id: &str,
    skill_name: &str,
    skill_dir: &Path,
    recording: &record::Recording,
    strict: bool,
) -> Result<()> {
    let span_path = paths::span_file(id)?;
    let events = span::read_span(&span_path)
        .with_context(|| format!("could not read span {}", span_path.display()))?;

    // The same faithful render `--fast` would install, marked unauthored. It is a valid
    // floor (real steps, secrets redacted); the brief asks the agent to raise it.
    let content = mark_unauthored(
        skill_name,
        &render_complete_skill(skill_name, recording, &events),
    );
    // Gated as a draft (lenient practicality), though the faithful render passes anyway;
    // the full Security axis still runs — it is a file a human may open.
    let ctx = validate::ValidationCtx::new(true, strict);
    gate_or_bail(&content, &ctx)?;

    paths::ensure_not_symlinked(skill_dir)?;
    std::fs::create_dir_all(skill_dir)?;
    let skill_path = skill_dir.join("SKILL.md");
    warn_on_overwrite(&skill_path);
    std::fs::write(&skill_path, &content)?;

    note_skill_written(skill_name, &skill_path, id, catalog::STATUS_DRAFT);

    let home = paths::home_dir().map(|p| p.display().to_string());
    let span_ref =
        validate::generalize_session_text(&span_path.display().to_string(), home.as_deref());
    print_authoring_brief(id, &skill_path, &span_ref, meaningful_steps(&events).len());
    Ok(())
}

/// The path to a recording's ephemeral authoring frames, if any were kept
/// (`capture.keep_frames`). `None` when the directory is absent or empty.
fn frames_hint(rec_id: &str) -> Option<String> {
    let dir = paths::frames_dir(rec_id).ok()?;
    let has_any = std::fs::read_dir(&dir).ok()?.next().is_some();
    has_any.then(|| tilde(&dir))
}

/// Inserts a discreet machine marker above the title so `warn_on_overwrite` (and a
/// later re-distill) recognizes an unauthored draft. Invisible in rendered Markdown;
/// the human-facing brief is printed, not embedded, so the draft stays a clean skill.
fn mark_unauthored(skill_name: &str, body: &str) -> String {
    body.replacen(
        &format!("# {skill_name}\n"),
        &format!("<!-- galdr:unauthored -->\n# {skill_name}\n"),
        1,
    )
}

/// The authoring brief: what galdr captured, and the judgment the agent must add to turn
/// a faithful replay into a skill worth reusing. This is where the intelligence galdr
/// deliberately does not fake enters the loop — the agent is the author, galdr the scribe.
fn print_authoring_brief(rec_id: &str, skill_path: &Path, span_ref: &str, steps: usize) {
    println!(
        "{} faithful draft of {steps} step(s) written — now author the real skill.",
        crate::style::accent("●")
    );
    println!("  draft:  {}", tilde(skill_path));
    println!("  span:   {span_ref}   (full tool_input/tool_response, one JSON line per step)");
    if let Some(frames) = frames_hint(rec_id) {
        println!(
            "  frames: {frames}   (screenshots of each step — read them to author with vision; purged on install)"
        );
    }
    println!();
    println!("galdr captured WHAT ran; you supply WHY. Read the span and rewrite the draft:");
    println!("  - Description / When to use: the real problem it solves and when to reach for");
    println!("    it, so matching is precise — not a restatement of the steps.");
    println!("  - Inputs: promote the values that vary to named parameters with judgment;");
    println!("    drop the incidental literals.");
    println!("  - Steps: name each command's intent, keep the essential order, group the noise.");
    println!("  - Verification: how to know it worked (each step's tool_response gives hints).");
    println!("  - Gotchas: preconditions and what to do when a step fails.");
    println!("Keep the Provenance block; the content gate runs again on install.");
    println!();
    println!(
        "  install your version:  {}",
        crate::style::accent("galdr distill --from <your-file>")
    );
    println!(
        "  or accept this draft:  {}",
        crate::style::accent("galdr distill --fast")
    );
}

/// Autonomous distillation: a local MLX engine writes the finished skill from the
/// span. Falls back to the deterministic complete skill if the engine is unselected,
/// missing, or unreachable, or if its output fails validation — so `--auto` without a
/// model still installs a usable skill, never a dead-end draft. Always exits cleanly.
pub fn distill_auto(
    id: &str,
    engine_override: Option<&str>,
    strict: bool,
    name: Option<&str>,
) -> Result<()> {
    let recording = load_recording(id)?;
    let skill_name = skill_name_for(name, &recording);
    let skill_dir = paths::skill_dir(&skill_name)?;

    let config = Config::load()?;
    let kind = match engine_override {
        Some(value) => EngineKind::parse(value)?,
        None => EngineKind::parse(&config.engine)?,
    };

    let span_path = paths::span_file(id)?;
    let events = span::read_span(&span_path)
        .with_context(|| format!("could not read span {}", span_path.display()))?;

    if let Some(engine) = engine::build_engine(kind, &config) {
        if engine.detect() {
            let (system, user) = build_prompt(&recording, &events, &config);
            match engine.distill(&system, &user) {
                Ok(skill_md) => match validate_skill_md(&skill_md) {
                    Ok(()) => {
                        // A machine-generated skill must clear the same content gate as
                        // any other. If it does not (a leaked secret, a personal path),
                        // fall back to the deterministic complete skill rather than
                        // installing something the gate would reject.
                        let ctx = validate::ValidationCtx::new(false, strict);
                        if validate::validate_skill(&skill_md, &ctx).has_blocking(ctx.strict) {
                            eprintln!(
                                "generated skill failed the validation gate; writing a complete skill"
                            );
                        } else {
                            install_skill(&skill_name, &skill_dir, &skill_md, id, &ctx)?;
                            println!("Autonomous distillation complete (engine: {kind:?}).");
                            println!("Review the skill before use — it was machine-generated.");
                            return Ok(());
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "generated skill failed validation ({err}); writing a complete skill"
                        );
                    }
                },
                Err(err) => eprintln!("engine error ({err}); writing a complete skill"),
            }
        } else {
            eprintln!("autonomous engine not reachable; writing a complete skill");
        }
    } else {
        eprintln!("no autonomous engine available; writing a complete skill");
    }

    write_complete(id, &skill_name, &skill_dir, &recording, strict)
}

/// The validation gate, shared by every writer of the skills directory. Validates the
/// content, prints any findings, and refuses to install when something blocks: always
/// on an Error (a secret, a personal path, a broken skill), and on a Warn (a
/// documented danger, a tautological description) only under `--strict`. This is the
/// single checkpoint each writer runs immediately before its `fs::write`, so no skill
/// reaches disk — and no other agent reads it — without passing.
pub(crate) fn gate_or_bail(md: &str, ctx: &validate::ValidationCtx) -> Result<()> {
    let report = validate::validate_skill(md, ctx);
    if !report.is_empty() {
        eprint!("{report}");
    }
    if report.has_blocking(ctx.strict) {
        bail!(
            "skill failed the validation gate: {} blocking finding(s){}. Not installing.",
            report.blocking_count(ctx.strict),
            if ctx.strict { " (strict mode)" } else { "" }
        );
    }
    Ok(())
}

/// Abbreviates the user's home directory to `~` for friendlier, shareable output — so
/// an install line reads `~/.agents/skills/…` instead of a long personal path.
fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match paths::home_dir() {
        Some(home) => {
            let home = home.display().to_string();
            shown
                .strip_prefix(&home)
                .map(|rest| format!("~{rest}"))
                .unwrap_or(shown)
        }
        None => shown,
    }
}

/// The single sanctioned writer of the skills directory, shared by `--from` and
/// `--auto`. Runs the gate, writes the `SKILL.md`, and records its provenance
/// best-effort.
fn install_skill(
    skill_name: &str,
    skill_dir: &Path,
    content: &str,
    rec_id: &str,
    ctx: &validate::ValidationCtx,
) -> Result<()> {
    gate_or_bail(content, ctx)?;
    paths::ensure_not_symlinked(skill_dir)?;
    std::fs::create_dir_all(skill_dir)?;
    let skill_path = skill_dir.join("SKILL.md");
    warn_on_overwrite(&skill_path);
    std::fs::write(&skill_path, content)?;
    println!(
        "{} skill installed: {}",
        crate::style::green("✓"),
        tilde(&skill_path)
    );

    note_skill_written(skill_name, &skill_path, rec_id, catalog::STATUS_FINAL);
    // Authoring is done, so the ephemeral frames have served their purpose: purge the
    // pixels. They were scaffolding to produce the skill, never part of it.
    if let Ok(frames) = paths::frames_dir(rec_id) {
        let _ = std::fs::remove_dir_all(frames);
    }
    // A skill the harness can't find is useless: make the finished skill discoverable
    // in every installed harness (Claude Code, Codex, Cursor) by linking it in.
    report_discoverability(skill_name);
    Ok(())
}

/// Links the just-installed skill into each detected harness and prints where it
/// is now discoverable. Best-effort: linking never blocks an install.
fn report_discoverability(skill_name: &str) {
    let Ok(results) = crate::link::link_skill(skill_name) else {
        return;
    };
    let reached: Vec<&str> = results
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                crate::link::LinkStatus::Linked
                    | crate::link::LinkStatus::AlreadyLinked
                    | crate::link::LinkStatus::SameRoot
            )
        })
        .map(|r| r.harness.as_str())
        .collect();
    if !reached.is_empty() {
        println!("Discoverable in: {}", reached.join(", "));
    }
    for r in &results {
        if matches!(
            r.status,
            crate::link::LinkStatus::Conflict | crate::link::LinkStatus::Failed
        ) {
            eprintln!(
                "warning: could not link into {} ({}): {}",
                r.harness,
                r.status.as_str(),
                r.link_path
            );
        }
    }
}

/// Warns before a write replaces an existing `SKILL.md`. If that file looked like a
/// finished (refined) skill rather than a draft, the warning is loud: overwriting
/// it silently would throw away real work. galdr is the only writer of the skills
/// directory, so this is the one place to catch an accidental clobber.
fn warn_on_overwrite(skill_path: &Path) {
    let Ok(existing) = std::fs::read_to_string(skill_path) else {
        return; // nothing to replace
    };
    if existing.contains("galdr:unauthored")
        || existing.contains("[galdr DRAFT]")
        || existing.contains("TODO(agent)")
    {
        println!(
            "note: replacing the existing draft at {}",
            skill_path.display()
        );
    } else {
        eprintln!(
            "warning: overwriting a finished skill at {} — its refinements will be lost",
            skill_path.display()
        );
    }
}

fn note_skill_written(skill_name: &str, skill_path: &Path, rec_id: &str, status: &str) {
    let skill_path_string = skill_path.display().to_string();
    let installed_at = record::now_rfc3339();
    let _ = catalog::sync_installed_skill(
        skill_name,
        Some(rec_id),
        &skill_path_string,
        Some(&installed_at),
        status,
    );

    crate::ipc::notify_best_effort(&crate::ipc::Request::SkillInstalled {
        skill_name: skill_name.to_string(),
        rec_id: rec_id.to_string(),
        skill_path: skill_path_string,
        status: status.to_string(),
    });
}
