//! Self-update: check crates.io for a newer galdr and install it on request.
//!
//! galdr is local-first with zero background network. The **only** time it reaches
//! the network on its own initiative is when the user explicitly asks — `galdr
//! upgrade`, `galdr upgrade --check`, and the update line in `galdr doctor`. Every
//! one of those touches crates.io through a short-timeout `curl` shell-out and fails
//! soft: no connection is a *note*, never an error.
//!
//! We shell out to `curl --max-time 3` rather than pull in an HTTP client. The only
//! HTTP dependency in the tree (`reqwest`) is gated behind the optional `mlx` feature
//! and loopback-only by design, so a default build has no client at all — and adding
//! a TLS stack just to read one index line would be pure weight against the
//! local-first ethos. `curl` is present on every macOS and virtually every Linux; if
//! it is missing, that is simply treated as "offline".
//!
//! The crates.io *sparse* index serves one NDJSON line per published version at
//! `https://index.crates.io/ga/ld/galdr` (the `ga/ld/` shard is derived from the
//! crate name). We keep the greatest non-yanked semver and compare it against this
//! binary's compile-time `CARGO_PKG_VERSION`.

use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::{daemon, ipc};

/// The crates.io sparse-index URL for galdr. One JSON line per published version.
const INDEX_URL: &str = "https://index.crates.io/ga/ld/galdr";

/// How long `curl` may spend on the whole transfer before it is treated as offline.
/// Short on purpose: an update check must never make `doctor` feel slow.
const CURL_MAX_TIME: &str = "3";

/// A minimal semantic version: `major.minor.patch` with an optional pre-release tag.
///
/// crates.io release versions are plain `x.y.z`, but we parse (and order) a
/// pre-release suffix too so a `-rc` build never sorts above its own release. Build
/// metadata (`+…`) is stripped: semver says it does not affect precedence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl SemVer {
    /// Parses `major.minor.patch[-pre][+build]`. Returns `None` for anything that is
    /// not three dotted integers, so a stray or malformed index line is simply
    /// ignored rather than crashing the check.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        // Peel off pre-release (`-`) first, then build metadata (`+`) from either part.
        let (core, pre) = match raw.split_once('-') {
            Some((core, rest)) => {
                let pre = rest.split('+').next().unwrap_or(rest);
                if pre.is_empty() {
                    return None;
                }
                (core, Some(pre.to_string()))
            }
            None => (raw.split('+').next().unwrap_or(raw), None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None; // more than three components is not a version we understand.
        }
        Some(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                // A release outranks any pre-release of the same core version.
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                // Both pre-release: a lexical compare is enough for galdr's scheme.
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// One line of the crates.io sparse index. We only need the version and its yanked
/// flag; every other field (deps, checksum, features) is ignored.
#[derive(Deserialize)]
struct IndexEntry {
    vers: String,
    #[serde(default)]
    yanked: bool,
}

/// The greatest non-yanked version in a crates.io sparse-index body. Tolerates stray
/// or unparseable lines (a captive portal that returns HTML yields *no* usable
/// version, which surfaces as an error the caller can treat as "skip").
pub fn parse_index(raw: &str) -> Result<SemVer> {
    let mut best: Option<SemVer> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<IndexEntry>(line) else {
            continue;
        };
        if entry.yanked {
            continue;
        }
        let Some(version) = SemVer::parse(&entry.vers) else {
            continue;
        };
        if best.as_ref().is_none_or(|current| version > *current) {
            best = Some(version);
        }
    }
    best.context("crates.io index carried no usable galdr version")
}

/// This binary's own version, from the compile-time `CARGO_PKG_VERSION`. Infallible:
/// Cargo guarantees a valid semver here.
fn current_version() -> SemVer {
    SemVer::parse(env!("CARGO_PKG_VERSION")).expect("galdr's own version is valid semver")
}

/// Fetches the raw sparse-index body, or `None` when the network cannot be reached.
///
/// `GALDR_INDEX_FILE` (read a local file instead of the network) and
/// `GALDR_INDEX_URL` (override the crates.io URL) are test/debug affordances in the
/// same spirit as `GALDR_ROOT`: they keep the update path hermetic and let an
/// offline case be simulated deterministically. A missing/unreadable file, a `curl`
/// failure, a timeout, or a missing `curl` binary all collapse to `None` — offline.
fn fetch_index() -> Option<String> {
    if let Some(file) = std::env::var_os("GALDR_INDEX_FILE") {
        // A missing fixture is exactly the "offline" signal tests want.
        return std::fs::read_to_string(file).ok();
    }
    let url = std::env::var("GALDR_INDEX_URL").unwrap_or_else(|_| INDEX_URL.to_string());
    curl_get(&url)
}

/// A single, short-timeout GET via `curl`. Any failure — spawn error (no curl),
/// non-zero exit (network down, HTTP error under `-f`), or non-UTF-8 body — is
/// `None`, i.e. offline.
fn curl_get(url: &str) -> Option<String> {
    let output = Command::new("curl")
        .args(["--max-time", CURL_MAX_TIME, "-sfL", url])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The outcome of comparing the installed galdr against the crates.io index.
#[derive(Debug, PartialEq, Eq)]
pub enum LatestCheck {
    /// The installed version matches the newest published one.
    UpToDate { current: SemVer },
    /// A newer version is published.
    Newer { current: SemVer, latest: SemVer },
    /// The local build is newer than anything on crates.io — the normal state when
    /// running from a clone whose version bump has not been published yet.
    LocalAhead { current: SemVer, latest: SemVer },
    /// The index could not be reached (or read). Never an error, by design.
    Offline,
}

/// Compares this binary against the newest published galdr. Offline is a variant, not
/// an error; a genuinely malformed index (reachable but not parseable) is the only
/// error case, which callers may still choose to treat softly.
pub fn check_latest() -> Result<LatestCheck> {
    let current = current_version();
    let Some(raw) = fetch_index() else {
        return Ok(LatestCheck::Offline);
    };
    let latest = parse_index(&raw)?;
    Ok(match latest.cmp(&current) {
        Ordering::Equal => LatestCheck::UpToDate { current },
        Ordering::Greater => LatestCheck::Newer { current, latest },
        Ordering::Less => LatestCheck::LocalAhead { current, latest },
    })
}

/// Where `galdr upgrade` installs from.
#[derive(Debug, PartialEq, Eq)]
pub enum InstallSource {
    /// `cargo install galdr` — the published crate (default).
    Crates,
    /// `cargo install --path <dir>` — a local clone, the operator's usual path.
    Path(PathBuf),
}

impl InstallSource {
    /// Interprets the `--from` values: absent or `crates` → crates.io; `path <dir>` →
    /// a local clone. Anything else is a usage error naming the exact accepted forms.
    pub fn parse(from: Option<Vec<String>>) -> Result<Self> {
        let Some(values) = from else {
            return Ok(Self::Crates);
        };
        match values.as_slice() {
            [kind] if kind == "crates" => Ok(Self::Crates),
            [kind, dir] if kind == "path" => Ok(Self::Path(PathBuf::from(dir))),
            [kind] if kind == "path" => {
                bail!("`--from path` needs a directory: `galdr upgrade --from path <dir>`")
            }
            _ => bail!("invalid --from; use `--from crates` (default) or `--from path <dir>`"),
        }
    }
}

/// Entry point for `galdr upgrade [--check] [--from …]`. Returns the process exit
/// code: `0` for up to date / local-ahead / offline / a successful install, `10` for
/// `--check` when a newer version exists (a distinct, script-friendly signal). A
/// genuine failure (bad `--from`, missing `cargo`, a failed install) is an `Err`,
/// which the caller maps to exit `1`.
pub fn run(check: bool, from: Option<Vec<String>>) -> Result<i32> {
    // Validate the source up front so a bad `--from` fails fast, before any network.
    let source = InstallSource::parse(from)?;

    match check_latest()? {
        LatestCheck::Offline => {
            println!("update check skipped (offline; could not reach crates.io)");
            Ok(0)
        }
        LatestCheck::UpToDate { current } => {
            println!("galdr {current} is up to date");
            Ok(0)
        }
        LatestCheck::LocalAhead { current, latest } => {
            println!("local build v{current} ahead of crates.io (v{latest})");
            Ok(0)
        }
        LatestCheck::Newer { current, latest } => {
            if check {
                println!("galdr {latest} available (you have {current}) — run galdr upgrade");
                return Ok(10);
            }
            println!("galdr {current} → {latest}: upgrading via cargo install…");
            install(&source)?;
            println!("galdr upgraded to {latest}");
            restart_daemon_if_stale(&latest);
            Ok(0)
        }
    }
}

/// Runs the actual `cargo install`, inheriting stdio so the build streams live. A
/// missing `cargo` is reported with an actionable message rather than a raw OS error.
fn install(source: &InstallSource) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("install");
    match source {
        InstallSource::Crates => {
            cmd.args(["galdr", "--locked", "--force"]);
        }
        InstallSource::Path(dir) => {
            cmd.arg("--path").arg(dir).args(["--locked", "--force"]);
        }
    }
    let status = cmd.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "cargo not found on PATH; install Rust (https://rustup.rs), then re-run `galdr upgrade`"
            )
        } else {
            anyhow!("could not run cargo install: {e}")
        }
    })?;
    if !status.success() {
        bail!(
            "cargo install failed (exit {})",
            status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string())
        );
    }
    Ok(())
}

/// After a successful install the on-disk binary is new but any running daemon still
/// serves the old one over the control socket — the exact skew `galdr doctor` warns
/// about. If a daemon is running on a different (or unknown) version, restart it the
/// way the operator does: stop it, then relaunch detached. A running daemon that
/// already reports the new version is left alone.
fn restart_daemon_if_stale(latest: &SemVer) {
    let Ok(ipc::Response::Pong { version }) = ipc::query(&ipc::Request::Ping) else {
        // No daemon answering: nothing to restart.
        return;
    };
    let already_current = version
        .as_deref()
        .and_then(SemVer::parse)
        .is_some_and(|running| running == *latest);
    if already_current {
        println!("daemon already running {latest}; no restart needed");
        return;
    }
    let was = version.as_deref().unwrap_or("unknown");
    println!("restarting daemon (was {was}) so it runs {latest}…");
    if let Err(e) = restart_daemon() {
        eprintln!(
            "warning: could not restart the daemon: {e:#}\n\
             restart it yourself: galdr daemon stop && galdr daemon"
        );
    }
}

/// Stops the running daemon and relaunches it detached. Reuses the daemon's own
/// detach path (own process group, null stdio) — the same thing `galdr daemon
/// --detach` does — so the freshly installed binary now on disk becomes the daemon.
fn restart_daemon() -> Result<()> {
    // Ask the old daemon to shut down; ignore the reply (it may already be gone).
    let _ = ipc::query(&ipc::Request::Shutdown);
    // Wait for it to release the socket before we relaunch — otherwise the new
    // daemon's single-instance probe would see the old one and bow out.
    for _ in 0..40 {
        if ipc::query(&ipc::Request::Ping).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    daemon::run(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parses_the_common_shapes() {
        let v = SemVer::parse("0.15.0").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (0, 15, 0));
        assert!(v.pre.is_none());
        // Build metadata is stripped; a pre-release tag is retained.
        assert_eq!(
            SemVer::parse("1.2.3+abc").unwrap(),
            SemVer::parse("1.2.3").unwrap()
        );
        assert_eq!(
            SemVer::parse("1.2.3-rc.1+build").unwrap().pre.as_deref(),
            Some("rc.1")
        );
        // Garbage and wrong arity are rejected, not panicked on.
        assert!(SemVer::parse("not.a.version").is_none());
        assert!(SemVer::parse("1.2").is_none());
        assert!(SemVer::parse("1.2.3.4").is_none());
        assert!(SemVer::parse("1.2.3-").is_none());
    }

    #[test]
    fn semver_orders_including_local_ahead_and_prerelease() {
        let older = SemVer::parse("0.14.2").unwrap();
        let current = SemVer::parse("0.15.0").unwrap();
        let newer = SemVer::parse("0.15.1").unwrap();
        // The published index is behind a local build: local is greater (local-ahead).
        assert!(current > older);
        // A real update: the index is ahead.
        assert!(newer > current);
        // Equality is reflexive across a fresh parse.
        assert_eq!(current, SemVer::parse("0.15.0").unwrap());
        // Cross-component ordering, not lexical string ordering (0.9.0 < 0.10.0).
        assert!(SemVer::parse("0.10.0").unwrap() > SemVer::parse("0.9.0").unwrap());
        // A release outranks its own pre-release.
        assert!(current > SemVer::parse("0.15.0-rc.1").unwrap());
    }

    #[test]
    fn parse_index_picks_the_greatest_non_yanked_version() {
        // A newer-but-yanked release must not win; the greatest live version does,
        // regardless of line order. A malformed line is tolerated.
        let raw = concat!(
            r#"{"name":"galdr","vers":"0.14.0","deps":[],"cksum":"a","features":{},"yanked":false}"#,
            "\n",
            r#"{"name":"galdr","vers":"0.15.0","deps":[],"cksum":"b","features":{},"yanked":false}"#,
            "\n",
            r#"{"name":"galdr","vers":"0.16.0","deps":[],"cksum":"c","features":{},"yanked":true}"#,
            "\n",
            "not json at all\n",
            r#"{"name":"galdr","vers":"0.14.2","deps":[],"cksum":"d","features":{},"yanked":false}"#,
            "\n",
        );
        assert_eq!(parse_index(raw).unwrap(), SemVer::parse("0.15.0").unwrap());
    }

    #[test]
    fn parse_index_errors_when_nothing_usable() {
        // A reachable-but-garbage body (e.g. a captive portal) yields no version.
        assert!(parse_index("<html>hi</html>\n").is_err());
        // Every version yanked → nothing installable.
        assert!(parse_index(r#"{"vers":"1.0.0","yanked":true}"#).is_err());
    }

    #[test]
    fn install_source_parses_from_flag_values() {
        assert_eq!(InstallSource::parse(None).unwrap(), InstallSource::Crates);
        assert_eq!(
            InstallSource::parse(Some(vec!["crates".into()])).unwrap(),
            InstallSource::Crates
        );
        assert_eq!(
            InstallSource::parse(Some(vec!["path".into(), "/tmp/galdr".into()])).unwrap(),
            InstallSource::Path(PathBuf::from("/tmp/galdr"))
        );
        // `path` with no directory, and an unknown source, are usage errors.
        assert!(InstallSource::parse(Some(vec!["path".into()])).is_err());
        assert!(InstallSource::parse(Some(vec!["bogus".into()])).is_err());
    }
}
