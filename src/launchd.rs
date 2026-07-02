//! macOS launchd lifecycle for the galdr daemon.
//!
//! The daemon is otherwise started by hand (`nohup galdr daemon &`): it dies on
//! logout, never comes back after a reboot, and — as happened in the field — can sit
//! for days serving a stale build. A user LaunchAgent fixes all three: launchd starts
//! it at login (`RunAtLoad`), restarts it if it crashes (`KeepAlive`), and gives a
//! single, inspectable source of truth for whether it is managed.
//!
//! This is macOS-only. Linux has no launchd, so `install`/`uninstall` bail with a
//! clear message and `doctor` stays quiet about it.
//!
//! `launchctl` is invoked through [`launchctl`], whose binary can be overridden with
//! `GALDR_LAUNCHCTL` (a test/debug affordance in the same spirit as `GALDR_ROOT`):
//! the plist writing, path derivation, and CLI wiring are all exercised without
//! touching the real service database.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::{daemon, ipc, paths};

/// The LaunchAgent label. Also the plist basename (`dev.galdr.daemon.plist`) and the
/// service name under the GUI domain (`gui/<uid>/dev.galdr.daemon`).
pub const LABEL: &str = "dev.galdr.daemon";

/// Whether this build targets macOS. A function (not a bare `cfg!`) so callers read as
/// runtime checks and clippy does not fold the branch to a constant.
fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Gate for the platform-specific commands. launchd exists only on macOS.
fn ensure_macos() -> Result<()> {
    if is_macos() {
        Ok(())
    } else {
        bail!(
            "launchd is macOS-only; `galdr daemon install`/`uninstall` are unavailable on this platform"
        )
    }
}

/// The user LaunchAgent plist: `~/Library/LaunchAgents/dev.galdr.daemon.plist`.
fn plist_path() -> Result<PathBuf> {
    let home = paths::home_dir().context("could not determine the home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// This user's numeric uid, read from the owner of the home directory — dependency-free
/// (no libc), and correct for the user LaunchAgent domain `gui/<uid>`.
fn uid() -> Result<u32> {
    let home = paths::home_dir().context("could not determine the home directory")?;
    Ok(std::fs::metadata(&home)
        .with_context(|| format!("could not stat {}", home.display()))?
        .uid())
}

/// The GUI domain target for this user: `gui/<uid>`.
fn gui_domain() -> Result<String> {
    Ok(format!("gui/{}", uid()?))
}

/// The service target for the daemon: `gui/<uid>/dev.galdr.daemon`.
fn gui_service() -> Result<String> {
    Ok(format!("gui/{}/{LABEL}", uid()?))
}

/// The `launchctl` binary, overridable with `GALDR_LAUNCHCTL` for hermetic tests.
fn launchctl_bin() -> String {
    std::env::var("GALDR_LAUNCHCTL").unwrap_or_else(|_| "launchctl".to_string())
}

/// Runs `launchctl <args>`, returning whether it exited 0. Output is discarded (a
/// `launchctl print` dump is large and uninteresting here). Errors only when the
/// binary cannot be spawned at all — which the probes treat as "not managed".
fn launchctl(args: &[&str]) -> Result<bool> {
    let status = Command::new(launchctl_bin())
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("could not run launchctl")?;
    Ok(status.success())
}

/// Escapes the five XML predefined entities so an odd character in a path can never
/// produce a malformed plist.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Renders the LaunchAgent plist. Pure and total, so its content and paths are unit
/// tested without any launchd or filesystem side effects. `program` is the absolute
/// path to the galdr binary launchd should run as `<program> daemon`.
fn render_plist(program: &Path, out_log: &Path, err_log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
</dict>
</plist>
"#,
        label = LABEL,
        program = xml_escape(&program.display().to_string()),
        out = xml_escape(&out_log.display().to_string()),
        err = xml_escape(&err_log.display().to_string()),
    )
}

/// Whether the plist file exists. Cheap and HOME-scoped, so it also acts as the guard
/// that keeps `launchctl` from being invoked when there is plainly nothing installed.
fn plist_exists() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

/// Whether launchd currently has the job loaded (`launchctl print` exits 0). Only
/// consulted once the plist exists, so a fresh machine never shells out.
fn job_loaded() -> bool {
    let Ok(service) = gui_service() else {
        return false;
    };
    launchctl(&["print", &service]).unwrap_or(false)
}

/// Whether the daemon is managed by launchd: macOS, the plist is present, and the job
/// is loaded. `false` off macOS or when nothing is installed — checked cheaply first.
pub fn is_managed() -> bool {
    is_macos() && plist_exists() && job_loaded()
}

/// Management state for callers that must distinguish "not macOS" (`None`) from
/// managed/unmanaged (`Some(bool)`), e.g. `doctor`.
pub fn management() -> Option<bool> {
    if is_macos() { Some(is_managed()) } else { None }
}

/// A one-line launchd summary for `galdr daemon status`. `None` off macOS (there is no
/// launchd to report on). On macOS it names the three states: managed, installed but
/// not loaded, or unmanaged.
pub fn status_line() -> Option<String> {
    if !is_macos() {
        return None;
    }
    if !plist_exists() {
        return Some("launchd: unmanaged — run galdr daemon install".to_string());
    }
    if job_loaded() {
        Some("launchd: managed (auto-starts at login, restarts on crash)".to_string())
    } else {
        Some("launchd: installed but not loaded — run galdr daemon install".to_string())
    }
}

/// Restarts the launchd-managed daemon in place: `launchctl kickstart -k` kills the
/// running instance and starts a fresh one from the (already loaded) job definition,
/// which points at the on-disk binary — so after `galdr upgrade` the new build takes
/// over without a manual stop/start.
pub fn kickstart() -> Result<()> {
    ensure_macos()?;
    let service = gui_service()?;
    if !launchctl(&["kickstart", "-k", &service])? {
        bail!("launchctl kickstart failed for {service}");
    }
    Ok(())
}

/// Installs the LaunchAgent: write the plist, create the log directory, and load
/// (or reload) the job. If a loose `nohup` daemon is running it is stopped first, so
/// launchd does not end up racing a second instance for the control socket.
pub fn install() -> Result<()> {
    ensure_macos()?;

    // Decide bootstrap-vs-reload from the state *before* we rewrite the plist. A fresh
    // machine has no plist, so `is_managed` short-circuits without touching launchctl.
    let was_managed = is_managed();

    let program = std::env::current_exe().context("could not find the galdr executable")?;
    let out_log = paths::daemon_out_log()?;
    let err_log = paths::daemon_err_log()?;
    std::fs::create_dir_all(paths::logs_dir()?).context("could not create the daemon log dir")?;

    let plist = plist_path()?;
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent).context("could not create ~/Library/LaunchAgents")?;
    }
    std::fs::write(&plist, render_plist(&program, &out_log, &err_log))
        .with_context(|| format!("could not write {}", plist.display()))?;
    println!("wrote LaunchAgent: {}", plist.display());

    let service = gui_service()?;
    if was_managed {
        // Already loaded from a prior install: restart cleanly so the (possibly new)
        // binary and plist take effect.
        if !launchctl(&["kickstart", "-k", &service])? {
            bail!("launchctl kickstart failed for {service}");
        }
        println!("reloaded {LABEL} (launchctl kickstart -k)");
    } else {
        // First load: stop any loose daemon so we do not leave two running, then hand
        // the socket to launchd.
        if matches!(
            ipc::query(&ipc::Request::Ping),
            Ok(ipc::Response::Pong { .. })
        ) {
            daemon::stop_and_wait();
            println!("stopped the running (unmanaged) daemon");
        }
        let domain = gui_domain()?;
        let plist_str = plist
            .to_str()
            .context("plist path is not valid UTF-8")?
            .to_string();
        if !launchctl(&["bootstrap", &domain, &plist_str])? {
            bail!(
                "launchctl bootstrap failed for {service}; inspect it with `launchctl print {service}`"
            );
        }
        println!("loaded {LABEL} (launchctl bootstrap {domain})");
    }

    println!("galdr daemon is now managed by launchd (auto-starts at login, restarts on crash)");
    Ok(())
}

/// Uninstalls the LaunchAgent: unload the job (tolerant if it is not loaded) and
/// remove the plist. Logs and state under `~/.galdr` are deliberately left in place.
pub fn uninstall() -> Result<()> {
    ensure_macos()?;

    let service = gui_service()?;
    // Tolerant: booting out a job that is not loaded is a no-op, not a failure.
    let _ = launchctl(&["bootout", &service]);
    println!("unloaded {LABEL} (launchctl bootout)");

    let plist = plist_path()?;
    if plist.exists() {
        std::fs::remove_file(&plist)
            .with_context(|| format!("could not remove {}", plist.display()))?;
        println!("removed {}", plist.display());
    } else {
        println!("no LaunchAgent plist to remove ({})", plist.display());
    }
    println!("logs and recordings under ~/.galdr are left in place");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_has_the_required_keys_and_paths() {
        let plist = render_plist(
            Path::new("/Users/x/.cargo/bin/galdr"),
            Path::new("/Users/x/.galdr/logs/daemon.out.log"),
            Path::new("/Users/x/.galdr/logs/daemon.err.log"),
        );
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("<string>dev.galdr.daemon</string>"));
        // ProgramArguments is the absolute binary followed by the `daemon` subcommand.
        assert!(plist.contains("<string>/Users/x/.cargo/bin/galdr</string>"));
        assert!(plist.contains("<string>daemon</string>"));
        // Survives login and crashes.
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n    <true/>"));
        // Logs are wired to the galdr log dir.
        assert!(plist.contains("<string>/Users/x/.galdr/logs/daemon.out.log</string>"));
        assert!(plist.contains("<string>/Users/x/.galdr/logs/daemon.err.log</string>"));
        // Well-formed header.
        assert!(plist.starts_with("<?xml version=\"1.0\""));
        assert!(plist.contains("<!DOCTYPE plist PUBLIC"));
    }

    #[test]
    fn plist_escapes_xml_metacharacters_in_paths() {
        // A path with `&`/`<` must not break the plist XML.
        let plist = render_plist(
            Path::new("/tmp/a&b/galdr"),
            Path::new("/tmp/o<ut.log"),
            Path::new("/tmp/err.log"),
        );
        assert!(plist.contains("/tmp/a&amp;b/galdr"));
        assert!(plist.contains("/tmp/o&lt;ut.log"));
        assert!(!plist.contains("/tmp/a&b/galdr"));
    }

    #[test]
    fn label_is_the_reverse_dns_id() {
        assert_eq!(LABEL, "dev.galdr.daemon");
    }
}
