fn main() {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        // `galdr` with no subcommand is a human at a terminal — show a friendly overview,
        // not a clap usage error.
        exit_on_error(cmd_overview());
        return;
    };
    match command {
        Commands::Hook => {
            // The sensor must NEVER propagate a failure to the agent session. We
            // silence the panic message, catch any unwind, and always exit 0, no
            // matter what happens inside `hook::run`.
            std::panic::set_hook(Box::new(|_| {}));
            let _ = std::panic::catch_unwind(|| {
                let _ = hook::run();
            });
            std::process::exit(0);
        }
        Commands::Rec { action } => {
            let result = match action {
                RecAction::Start { name } => record::start(name),
                RecAction::Stop { reference } => record::stop(reference.as_deref()),
                RecAction::Status => cmd_rec_status(),
            };
            exit_on_error(result);
        }
        Commands::Observe { action } => match action {
            ObserveAction::Synthetic { name, fixture } => {
                exit_on_error(observe::synthetic(name, fixture))
            }
            ObserveAction::Browser { action } => match action {
                BrowserObserveAction::Start {
                    name,
                    url,
                    browser,
                    no_open,
                    headless,
                } => exit_on_error(observe::browser_start(
                    name, url, browser, no_open, headless,
                )),
                BrowserObserveAction::Stop => exit_on_error(observe::browser_stop()),
                BrowserObserveAction::Status => exit_on_error(observe::browser_status()),
                BrowserObserveAction::Serve { rec_id } => {
                    exit_on_error(observe::browser_serve(&rec_id))
                }
            },
            ObserveAction::Mac { action } => match action {
                MacObserveAction::Start { name } => exit_on_error(observe_mac::mac_start(name)),
                MacObserveAction::Stop => exit_on_error(observe_mac::mac_stop()),
                MacObserveAction::Status => exit_on_error(observe_mac::mac_status()),
                MacObserveAction::Serve { rec_id } => {
                    exit_on_error(observe_mac::mac_serve(&rec_id))
                }
            },
        },
        Commands::Distill {
            reference,
            from,
            fast,
            auto,
            engine,
            name,
            strict,
        } => {
            let id = resolve_or_exit(reference.as_deref());
            if auto {
                exit_on_error(distill::distill_auto(
                    &id,
                    engine.as_deref(),
                    strict,
                    name.as_deref(),
                ))
            } else {
                exit_on_error(distill::distill(
                    &id,
                    from.as_deref(),
                    fast,
                    strict,
                    name.as_deref(),
                ))
            }
        }
        Commands::List { json } => exit_on_error(cmd_list(json)),
        Commands::Show { reference, json } => {
            exit_on_error(cmd_show(&resolve_or_exit(reference.as_deref()), json))
        }
        Commands::Judge { action } => exit_on_error(cmd_judge(action)),
        Commands::Skills { json } => exit_on_error(cmd_skills(json)),
        Commands::Harnesses { json } => exit_on_error(cmd_harnesses(json)),
        Commands::Link { skill, all, json } => exit_on_error(cmd_link(skill.as_deref(), all, json)),
        Commands::Rm { skill, force } => exit_on_error(remove::run(&skill, force)),
        Commands::Evaluations { skill, json } => {
            exit_on_error(cmd_evaluations(skill.as_deref(), json))
        }
        Commands::Outcome { action } => exit_on_error(cmd_outcome(action)),
        Commands::Regress { action } => exit_on_error(cmd_regress(action)),
        Commands::Tui => exit_on_error(tui::run()),
        Commands::Export {
            reference,
            out,
            include_raw,
            redact,
        } => exit_on_error(export::export_recording(
            &resolve_or_exit(reference.as_deref()),
            &out,
            include_raw,
            redact,
        )),
        Commands::Validate {
            skill,
            all,
            file,
            strict,
            json,
        } => exit_on_error(cmd_validate(
            skill.as_deref(),
            all,
            file.as_deref(),
            strict,
            json,
        )),
        Commands::Doctor => exit_on_error(doctor::run()),
        Commands::Upgrade { check, from } => match upgrade::run(check, from) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("error: {err:#}");
                std::process::exit(1);
            }
        },
        Commands::Setup { target } => match target {
            SetupTarget::Claude { check, print } => {
                if print {
                    setup::claude_print();
                }
                if check || !print {
                    exit_on_error(setup::claude_check());
                }
            }
            SetupTarget::Codex { check, print } => {
                if print {
                    setup::codex_print();
                }
                if check || !print {
                    exit_on_error(setup::codex_check());
                }
            }
            SetupTarget::Cursor { check, print } => {
                if print {
                    setup::cursor_print();
                }
                if check || !print {
                    exit_on_error(setup::cursor_check());
                }
            }
            SetupTarget::Skill { print } => {
                if print {
                    print!("{}", skill::render());
                } else {
                    exit_on_error(cmd_setup_skill());
                }
            }
        },
        Commands::Diff { a, b } => exit_on_error(cmd_diff(&a, &b)),
        Commands::Parametrize { a, b, emit } => {
            exit_on_error(parametrize::parametrize(&a, &b, emit))
        }
        Commands::Bench { skill, json } => exit_on_error(bench::run(skill.as_deref(), json)),
        Commands::Suggest {
            json,
            top,
            min_count,
        } => exit_on_error(suggest::run(json, top, min_count)),
        Commands::Reindex => exit_on_error(cmd_reindex()),
        Commands::Daemon { action, detach } => match action {
            Some(DaemonAction::Status) => exit_on_error(cmd_daemon_status()),
            Some(DaemonAction::Stop) => exit_on_error(cmd_daemon_stop()),
            Some(DaemonAction::Install) => exit_on_error(launchd::install()),
            Some(DaemonAction::Uninstall) => exit_on_error(launchd::uninstall()),
            None => exit_on_error(daemon::run(detach)),
        },
    }
}
