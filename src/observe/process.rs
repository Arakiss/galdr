fn find_browser() -> Option<PathBuf> {
    for path in [
        "/Applications/Google Chrome.app",
        "/Applications/Chromium.app",
        "/Applications/Brave Browser.app",
        "/Applications/Microsoft Edge.app",
    ] {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    for name in [
        "google-chrome",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
    ] {
        if let Some(path) = find_on_path(name) {
            return Some(path);
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn launch_browser(browser: &Path, session: &BrowserObserveSession) -> Result<u32> {
    let args = browser_args(session);
    let browser_path = browser_executable(browser);
    let child = std::process::Command::new(&browser_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not launch browser {}", browser_path.display()))?;
    Ok(child.id())
}

fn browser_args(session: &BrowserObserveSession) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", session.profile_dir.display()),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--remote-debugging-port={}", session.devtools_port),
        "--remote-allow-origins=*".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-sync".to_string(),
        "--disable-features=PasswordManagerOnboarding".to_string(),
        "--password-store=basic".to_string(),
        "--use-mock-keychain".to_string(),
    ];
    if session.headless {
        args.push("--headless=new".to_string());
        args.push("--disable-gpu".to_string());
    } else {
        args.push("--new-window".to_string());
    }
    args.push(session.url.clone());
    args
}

fn is_macos_app(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("app")
}

fn browser_executable(browser: &Path) -> PathBuf {
    if !is_macos_app(browser) {
        return browser.to_path_buf();
    }
    let Some(name) = browser.file_stem().and_then(|name| name.to_str()) else {
        return browser.to_path_buf();
    };
    let executable = browser.join("Contents").join("MacOS").join(name);
    if executable.exists() {
        executable
    } else {
        browser.to_path_buf()
    }
}

fn terminate_process(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
}

fn terminate_browser_profile(profile_dir: &Path) {
    #[cfg(unix)]
    {
        let marker = format!("--user-data-dir={}", profile_dir.display());
        let Ok(output) = std::process::Command::new("ps")
            .args(["-axo", "pid=,command="])
            .output()
        else {
            return;
        };
        let raw = String::from_utf8_lossy(&output.stdout);
        for line in raw.lines() {
            if !line.contains(&marker) {
                continue;
            }
            let Some(pid) = line
                .split_whitespace()
                .next()
                .and_then(|raw| raw.parse::<u32>().ok())
            else {
                continue;
            };
            terminate_process(Some(pid));
        }
    }
    #[cfg(not(unix))]
    let _ = profile_dir;
}
