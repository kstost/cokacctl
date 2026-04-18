use crate::core::{config::Config, download, platform, version, ProgressMsg, ProgressTx};

fn send(tx: &Option<ProgressTx>, msg: String) {
    if let Some(tx) = tx {
        tx.send(ProgressMsg::Log(msg)).ok();
    } else {
        println!("{}", msg);
    }
}

/// Best-effort restart after an update step failed while the service was
/// stopped — avoids leaving the user with a silently-down service.
fn try_restart_existing(tx: &Option<ProgressTx>) {
    let config = Config::load();
    let tokens = config.active_tokens();
    if tokens.is_empty() {
        return;
    }
    if let Some(existing) = platform::find_cokacdir() {
        dlog!("update", "Rollback: restarting with {}", existing.display());
        send(tx, "  Update failed — restarting service with existing binary...".into());
        let _ = crate::service::manager().start(&existing, &tokens);
    } else {
        dlog!("update", "Rollback: no existing binary found, cannot restart");
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

    if let Err(e) = download::download_to_path(&url, &binary_path, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }
    dlog!("update", "Download complete");
    send(tx, format!("  Updated to v{}", latest));

    if was_running {
        let config = Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("update", "Restarting service...");
            send(tx, "  Restarting service...".into());
            mgr.start(&binary_path, &tokens).ok();
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
    if let Err(e) = download::download_to_path(url, &tmp, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }

    send(tx, "  Requires elevated privileges. Using sudo...".into());
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
    dlog!("update", "Invoking: {}", sudo_label);
    let status = match cmd
        .args(["mv", &tmp.to_string_lossy(), &dest.to_string_lossy()])
        .status()
    {
        Ok(s) => {
            crate::core::debug::log_status("update", &sudo_label, &s);
            s
        }
        Err(e) => {
            dlog!("update", "sudo mv exec failed: {}", e);
            std::fs::remove_file(&tmp).ok();
            if was_running {
                try_restart_existing(tx);
            }
            return Err(format!("sudo mv failed: {}", e));
        }
    };

    if !status.success() {
        dlog!("update", "sudo mv failed");
        std::fs::remove_file(&tmp).ok();
        if was_running {
            try_restart_existing(tx);
        }
        return Err("sudo mv failed. Cannot update binary.".into());
    }

    // chmod +x; log failure since the old binary is already gone and the new
    // one may not be executable without this.
    let chmod_label = format!("sudo chmod +x {}", dest.display());
    dlog!("update", "Invoking: {}", chmod_label);
    match std::process::Command::new("sudo")
        .args(["chmod", "+x", &dest.to_string_lossy()])
        .status()
    {
        Ok(s) => {
            crate::core::debug::log_status("update", &chmod_label, &s);
            if s.success() {
                dlog!("update", "chmod +x succeeded");
            } else {
                dlog!("update", "chmod +x returned non-zero: {:?}", s.code());
                send(
                    tx,
                    format!(
                        "  Warning: sudo chmod +x returned exit {:?} — binary may not be executable",
                        s.code()
                    ),
                );
            }
        }
        Err(e) => {
            dlog!("update", "chmod +x failed to invoke: {}", e);
            send(
                tx,
                format!("  Warning: could not run sudo chmod +x: {}", e),
            );
        }
    }

    dlog!("update", "Binary updated via sudo");
    send(tx, "  Binary updated.".into());

    if was_running {
        let config = crate::core::config::Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("update", "Restarting service after sudo update...");
            send(tx, "  Restarting service...".into());
            let mgr = crate::service::manager();
            mgr.start(dest, &tokens).ok();
        }
    }

    Ok(())
}
