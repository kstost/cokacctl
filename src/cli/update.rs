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
    dlog!("update", "CLI run()");
    run_inner(&None).await
}

/// TUI entry point.
pub async fn run_bg(tx: ProgressTx) -> Result<(), String> {
    dlog!("update", "TUI run_bg()");
    let tx_opt = Some(tx);
    let result = run_inner(&tx_opt).await;
    if let Some(tx) = &tx_opt {
        tx.send(ProgressMsg::Done(result.clone())).ok();
    }
    dlog!("update", "run_bg() result: {:?}", result);
    result
}

async fn run_inner(tx: &Option<ProgressTx>) -> Result<(), String> {
    let os = platform::Os::detect();
    let arch = platform::Arch::detect();

    dlog!("update", "Finding installed binary...");
    let binary_path = platform::find_cokacdir().ok_or(
        "cokacdir not found. Run 'cokacctl install' first.".to_string(),
    )?;
    dlog!("update", "Found: {}", binary_path.display());

    let current = version::installed_version(&binary_path).ok_or(
        "Cannot determine installed cokacdir version.".to_string(),
    )?;
    dlog!("update", "Current version: {}", current);
    send(tx, format!("  Current version: v{}", current));

    send(tx, "  Checking for updates...".into());
    dlog!("update", "Fetching latest version...");
    let latest = version::latest_version()
        .await
        .ok_or("Cannot fetch latest version info.".to_string())?;
    dlog!("update", "Latest version: {}", latest);
    send(tx, format!("  Latest version:  v{}", latest));

    if !version::is_newer(&latest, &current) {
        dlog!("update", "Already up to date");
        send(tx, "  Already up to date!".into());
        return Ok(());
    }

    dlog!("update", "Updating {} -> {}", current, latest);
    send(tx, format!("  Updating v{} → v{}...", current, latest));

    let mgr = crate::service::manager();
    let was_running = mgr.status() == crate::service::ServiceStatus::Running || mgr.is_any_running();
    dlog!("update", "Service was_running: {}", was_running);
    if was_running {
        send(tx, "  Stopping service for update...".into());
        dlog!("update", "Stopping service...");
        mgr.stop().ok();
    }

    let url = platform::binary_download_url(os, arch);
    dlog!("update", "Download URL: {}", url);

    #[cfg(unix)]
    {
        let parent = binary_path.parent().unwrap_or(std::path::Path::new("/"));
        if !is_writable_dir(parent) {
            dlog!("update", "Not writable, using sudo");
            return update_with_sudo(&url, &binary_path, was_running, tx).await;
        }
    }

    download::download_to_path(&url, &binary_path, tx).await?;
    dlog!("update", "Download complete");
    send(tx, format!("  Updated to v{}", latest));

    if was_running {
        let config = Config::load();
        if !config.tokens.is_empty() {
            dlog!("update", "Restarting service...");
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
    dlog!("update", "update_with_sudo()");
    let tmp = std::env::temp_dir().join(format!("cokacdir_up_{}", std::process::id()));
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
        dlog!("update", "sudo mv failed");
        return Err("sudo mv failed. Cannot update binary.".into());
    }

    let _ = std::process::Command::new("sudo")
        .args(["chmod", "+x", &dest.to_string_lossy()])
        .status();

    dlog!("update", "Binary updated via sudo");
    send(tx, "  Binary updated.".into());

    if was_running {
        let config = crate::core::config::Config::load();
        if !config.tokens.is_empty() {
            dlog!("update", "Restarting service after sudo update...");
            send(tx, "  Restarting service...".into());
            let mgr = crate::service::manager();
            mgr.start(dest, &config.tokens).ok();
        }
    }

    Ok(())
}
