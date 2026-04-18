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

/// Best-effort service restart when an install step failed after the service
/// was stopped — keeps the user from ending up with a silently-down service.
/// Uses whatever binary is currently findable (old binary on plain failure,
/// or restored-from-.old binary on partial replacement).
fn try_restart_existing(tx: &Option<ProgressTx>) {
    let config = crate::core::config::Config::load();
    let tokens = config.active_tokens();
    if tokens.is_empty() {
        return;
    }
    if let Some(existing) = platform::find_cokacdir() {
        dlog!("install", "Rollback: restarting with {}", existing.display());
        send(tx, "  Install failed — restarting service with existing binary...".into());
        let _ = crate::service::manager().start(&existing, &tokens);
    } else {
        dlog!("install", "Rollback: no existing binary found, cannot restart");
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
    if let Err(e) = download::download_to_path(&url, &dest, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }

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
    if let Err(e) = download::download_to_path(url, &tmp, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }

    let mut cmd = std::process::Command::new("sudo");
    if tx.is_some() {
        cmd.arg("-n");
    }
    let sudo_label = format!(
        "sudo{} mv {} {}",
        if tx.is_some() { " -n" } else { "" },
        tmp.display(),
        dest.display()
    );
    dlog!("install", "Invoking: {}", sudo_label);
    let status = match cmd
        .args(["mv", &tmp.to_string_lossy(), &dest.to_string_lossy()])
        .status()
    {
        Ok(s) => {
            crate::core::debug::log_status("install", &sudo_label, &s);
            s
        }
        Err(e) => {
            dlog!("install", "sudo mv exec failed: {}", e);
            std::fs::remove_file(&tmp).ok();
            if was_running {
                try_restart_existing(tx);
            }
            return Err(format!("sudo mv failed: {}", e));
        }
    };

    let actual_path = if !status.success() {
        let fallback = platform::fallback_install_path();
        dlog!("install", "sudo failed, falling back to {}", fallback.display());
        send(tx, format!("  sudo failed. Installing to {} instead.", fallback.display()));
        if let Some(parent) = fallback.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Err(e) = std::fs::rename(&tmp, &fallback).or_else(|_| -> Result<(), String> {
            std::fs::copy(&tmp, &fallback).map(|_| ()).map_err(|e| format!("Copy failed: {}", e))
        }) {
            std::fs::remove_file(&tmp).ok();
            if was_running {
                try_restart_existing(tx);
            }
            return Err(e);
        }
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
