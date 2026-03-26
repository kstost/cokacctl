use crate::core::{config::Config, download, platform, version, ProgressMsg, ProgressTx};

fn send(tx: &Option<ProgressTx>, msg: String) {
    if let Some(tx) = tx {
        tx.send(ProgressMsg::Log(msg)).ok();
    } else {
        println!("{}", msg);
    }
}

/// CLI entry point.
pub async fn run() -> Result<(), String> {
    run_inner(&None).await
}

/// TUI entry point.
pub async fn run_bg(tx: ProgressTx) -> Result<(), String> {
    let tx_opt = Some(tx);
    let result = run_inner(&tx_opt).await;
    if let Some(tx) = &tx_opt {
        tx.send(ProgressMsg::Done(result.clone())).ok();
    }
    result
}

async fn run_inner(tx: &Option<ProgressTx>) -> Result<(), String> {
    let os = platform::Os::detect();
    let arch = platform::Arch::detect();

    // Find installed binary
    let binary_path = platform::find_cokacdir().ok_or(
        "cokacdir not found. Run 'cokacctl install' first.".to_string(),
    )?;

    // Get current version
    let current = version::installed_version(&binary_path).ok_or(
        "Cannot determine installed cokacdir version.".to_string(),
    )?;
    send(tx, format!("  Current version: v{}", current));

    // Check latest
    send(tx, "  Checking for updates...".into());
    let latest = version::latest_version()
        .await
        .ok_or("Cannot fetch latest version info.".to_string())?;
    send(tx, format!("  Latest version:  v{}", latest));

    if !version::is_newer(&latest, &current) {
        send(tx, "  Already up to date!".into());
        return Ok(());
    }

    send(tx, format!("  Updating v{} → v{}...", current, latest));

    // Check if service is running, stop it first
    let mgr = crate::service::manager();
    let was_running = mgr.status() == crate::service::ServiceStatus::Running;
    if was_running {
        send(tx, "  Stopping service for update...".into());
        mgr.stop().ok();
    }

    // Download new binary
    let url = platform::binary_download_url(os, arch);

    // On Unix, may need sudo for /usr/local/bin
    #[cfg(unix)]
    {
        let parent = binary_path.parent().unwrap_or(std::path::Path::new("/"));
        if !is_writable_dir(parent) {
            return update_with_sudo(&url, &binary_path, was_running, tx).await;
        }
    }

    download::download_to_path(&url, &binary_path, tx).await?;
    send(tx, format!("  Updated to v{}", latest));

    // Restart service if it was running
    if was_running {
        let config = Config::load();
        if !config.tokens.is_empty() {
            send(tx, "  Restarting service...".into());
            mgr.start(&binary_path, &config.tokens).ok();
        }
    }

    Ok(())
}

#[cfg(unix)]
fn is_writable_dir(path: &std::path::Path) -> bool {
    let test_file = path.join(".cokacctl_write_test");
    match std::fs::write(&test_file, b"") {
        Ok(_) => {
            std::fs::remove_file(&test_file).ok();
            true
        }
        Err(_) => false,
    }
}

#[cfg(unix)]
async fn update_with_sudo(
    url: &str,
    dest: &std::path::Path,
    was_running: bool,
    tx: &Option<ProgressTx>,
) -> Result<(), String> {
    let tmp = std::env::temp_dir().join("cokacdir_update_tmp");
    download::download_to_path(url, &tmp, tx).await?;

    send(tx, "  Requires elevated privileges. Using sudo...".into());
    let mut cmd = std::process::Command::new("sudo");
    if tx.is_some() {
        cmd.arg("-n");
    }
    let status = cmd
        .args(["mv", &tmp.to_string_lossy(), &dest.to_string_lossy()])
        .status()
        .map_err(|e| format!("sudo mv failed: {}", e))?;

    if !status.success() {
        return Err("sudo mv failed. Cannot update binary.".into());
    }

    let _ = std::process::Command::new("sudo")
        .args(["chmod", "+x", &dest.to_string_lossy()])
        .status();

    send(tx, "  Binary updated.".into());

    if was_running {
        let config = crate::core::config::Config::load();
        if !config.tokens.is_empty() {
            send(tx, "  Restarting service...".into());
            let mgr = crate::service::manager();
            mgr.start(dest, &config.tokens).ok();
        }
    }

    Ok(())
}
