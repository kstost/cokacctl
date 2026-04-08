use crate::core::{download, platform, ProgressMsg, ProgressTx};

const SHELL_FUNC: &str =
    r#"cokacdir() { command cokacdir "$@" && cd "$(cat ~/.cokacdir/lastdir 2>/dev/null || pwd)"; }"#;

fn send(tx: &Option<ProgressTx>, msg: String) {
    if let Some(tx) = tx {
        tx.send(ProgressMsg::Log(msg)).ok();
    } else {
        println!("{}", msg);
    }
}

/// CLI entry point (prints to stdout).
pub async fn run() -> Result<(), String> {
    dlog!("install", "CLI run()");
    run_inner(&None).await
}

/// TUI entry point (sends progress via channel).
pub async fn run_bg(tx: ProgressTx) -> Result<(), String> {
    dlog!("install", "TUI run_bg()");
    let tx_opt = Some(tx);
    let result = run_inner(&tx_opt).await;
    if let Some(tx) = &tx_opt {
        tx.send(ProgressMsg::Done(result.clone())).ok();
    }
    dlog!("install", "run_bg() result: {:?}", result);
    result
}

async fn run_inner(tx: &Option<ProgressTx>) -> Result<(), String> {
    let os = platform::Os::detect();
    let arch = platform::Arch::detect();
    let url = platform::binary_download_url(os, arch);
    let install_path = platform::default_install_path(os);

    dlog!("install", "OS: {:?}, Arch: {:?}", os, arch);
    dlog!("install", "URL: {}", url);
    dlog!("install", "Install path: {}", install_path.display());

    send(tx, format!("  Installing cokacdir ({}-{})...", os.as_str(), arch.as_str()));
    send(tx, format!("  Source: {}", url));
    send(tx, format!("  Target: {}", install_path.display()));

    // Stop service if running (binary may be locked, especially on Windows)
    dlog!("install", "Checking service status...");
    let mgr = crate::service::manager();
    let was_running = mgr.status() == crate::service::ServiceStatus::Running || mgr.is_any_running();
    dlog!("install", "Service was_running: {}", was_running);
    if was_running {
        send(tx, "  Stopping running service...".into());
        dlog!("install", "Stopping service...");
        mgr.stop().ok();
    }

    // Try default path, fallback if not writable
    let dest = if os != platform::Os::Windows {
        if let Some(parent) = install_path.parent() {
            if !is_writable(parent) {
                dlog!("install", "Default path not writable, trying sudo");
                send(tx, "  /usr/local/bin requires elevated privileges.".into());
                send(tx, "  Trying sudo...".into());
                return install_with_sudo(&url, &install_path, was_running, tx).await;
            }
        }
        install_path.clone()
    } else {
        install_path.clone()
    };

    dlog!("install", "Downloading to: {}", dest.display());
    download::download_to_path(&url, &dest, tx).await?;

    // Setup shell wrapper on Unix
    if os != platform::Os::Windows {
        dlog!("install", "Setting up shell wrapper...");
        setup_shell_wrapper_inner(tx);
    }

    send(tx, format!("  cokacdir installed at {}", dest.display()));
    dlog!("install", "Install complete at {}", dest.display());

    // Restart service if it was running
    if was_running {
        let config = crate::core::config::Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("install", "Restarting service...");
            send(tx, "  Restarting service...".into());
            mgr.start(&dest, &tokens).ok();
        }
    }

    Ok(())
}

async fn install_with_sudo(url: &str, dest: &std::path::Path, was_running: bool, tx: &Option<ProgressTx>) -> Result<(), String> {
    dlog!("install", "install_with_sudo()");
    let tmp = std::env::temp_dir().join(format!("cokacdir_dl_{}", std::process::id()));
    download::download_to_path(url, &tmp, tx).await?;

    let mut cmd = std::process::Command::new("sudo");
    if tx.is_some() {
        cmd.arg("-n");
    }
    let status = cmd
        .args(["mv", &tmp.to_string_lossy(), &dest.to_string_lossy()])
        .status()
        .map_err(|e| format!("sudo mv failed: {}", e))?;

    let actual_path = if !status.success() {
        let fallback = platform::fallback_install_path();
        dlog!("install", "sudo failed, falling back to {}", fallback.display());
        send(tx, format!("  sudo failed. Installing to {} instead.", fallback.display()));
        if let Some(parent) = fallback.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::rename(&tmp, &fallback).or_else(|_| {
            std::fs::copy(&tmp, &fallback).map(|_| ()).map_err(|e| format!("Copy failed: {}", e))
        })?;
        std::fs::remove_file(&tmp).ok();
        send(tx, format!("  cokacdir installed at {}", fallback.display()));
        send(tx, format!("  Note: Ensure {} is in your PATH", fallback.parent().unwrap_or(std::path::Path::new("~/.local/bin")).display()));
        fallback
    } else {
        dlog!("install", "sudo mv succeeded");
        send(tx, format!("  cokacdir installed at {}", dest.display()));
        dest.to_path_buf()
    };

    setup_shell_wrapper_inner(tx);

    if was_running {
        let config = crate::core::config::Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("install", "Restarting service after sudo install...");
            send(tx, "  Restarting service...".into());
            crate::service::manager().start(&actual_path, &tokens).ok();
        }
    }

    Ok(())
}

fn is_writable(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    let test_file = path.join(".cokacctl_write_test");
    match std::fs::write(&test_file, b"") {
        Ok(_) => {
            std::fs::remove_file(&test_file).ok();
            true
        }
        Err(_) => false,
    }
}

fn setup_shell_wrapper_inner(tx: &Option<ProgressTx>) {
    let config_path = match platform::shell_config_path() {
        Some(p) => p,
        None => return,
    };

    if config_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if content.contains("cokacdir()") {
                dlog!("install", "Shell wrapper already exists in {}", config_path.display());
                return;
            }
        }
    }

    let mut content = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    content.push_str("\n# cokacdir - cd to last directory on exit\n");
    content.push_str(SHELL_FUNC);
    content.push('\n');

    if std::fs::write(&config_path, &content).is_ok() {
        dlog!("install", "Shell wrapper added to {}", config_path.display());
        send(tx, format!("  Shell wrapper added to {}", config_path.display()));
    }
}
