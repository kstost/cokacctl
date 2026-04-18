use crate::core::platform::Os;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

pub fn run(skip_confirm: bool) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let os = Os::detect();

    // Collect paths and check what exists
    let (files, dirs) = collect_paths(&home, os);
    let existing_files: Vec<&PathBuf> = files.iter().filter(|p| p.exists()).collect();
    let existing_dirs: Vec<&PathBuf> = dirs.iter().filter(|p| p.exists()).collect();

    // Show what will happen
    println!();
    println!("  This will perform the following actions:");
    println!();
    match os {
        Os::MacOS => {
            println!("  1. Stop service (launchctl bootout)");
        }
        Os::Linux => {
            println!("  1. Stop service (systemctl --user stop & disable)");
        }
        Os::Windows => {
            println!("  1. Stop service (Task Scheduler delete & kill process)");
        }
    }
    if existing_files.is_empty() && existing_dirs.is_empty() {
        println!("  2. No files to remove (nothing found)");
    } else {
        println!("  2. Remove the following:");
        for path in &existing_files {
            println!("     {}", path.display());
        }
        for path in &existing_dirs {
            println!("     {}/", path.display());
        }
    }
    println!();

    // Ask for confirmation
    if !skip_confirm {
        print!("  Proceed? [y/N] ");
        std::io::stdout().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)
            .map_err(|e| format!("Failed to read input: {}", e))?;
        if !matches!(input.trim(), "y" | "Y") {
            println!("  Cancelled.");
            return Ok(());
        }
        println!();
    }

    // Phase 1: Stop and unregister services
    println!("  Stopping services...");
    match os {
        Os::MacOS => {
            #[cfg(unix)]
            {
                let uid = unsafe { libc::getuid() };
                let target = format!("gui/{}/com.cokacdir.server", uid);
                dlog!("uninstall", "launchctl bootout {}", target);
                match Command::new("launchctl").args(["bootout", &target]).output() {
                    Ok(out) => {
                        crate::core::debug::log_output("uninstall", "launchctl bootout", &out);
                        if out.status.success() {
                            dlog!("uninstall", "launchctl bootout: OK");
                            println!("    launchctl bootout: OK");
                        } else {
                            dlog!("uninstall", "launchctl bootout: skipped (non-success)");
                            println!("    launchctl bootout: skipped (not running)");
                        }
                    }
                    Err(e) => {
                        dlog!("uninstall", "launchctl bootout exec failed: {}", e);
                        println!("    launchctl bootout: skipped (not running)");
                    }
                }

                // Kill any externally running cokacdir processes
                dlog!("uninstall", "Killing all cokacdir processes via pkill...");
                match Command::new("pkill").arg("cokacdir").output() {
                    Ok(out) => {
                        crate::core::debug::log_output("uninstall", "pkill cokacdir", &out);
                        dlog!("uninstall", "pkill cokacdir exit={}", out.status.code().unwrap_or(-1));
                    }
                    Err(e) => {
                        dlog!("uninstall", "pkill cokacdir failed: {}", e);
                    }
                }
            }
        }
        Os::Linux => {
            dlog!("uninstall", "systemctl --user stop cokacdir");
            match Command::new("systemctl").args(["--user", "stop", "cokacdir"]).output() {
                Ok(out) => {
                    crate::core::debug::log_output("uninstall", "systemctl --user stop cokacdir", &out);
                    if out.status.success() {
                        println!("    systemctl stop: OK");
                    } else {
                        println!("    systemctl stop: skipped (not running)");
                    }
                }
                Err(e) => {
                    dlog!("uninstall", "systemctl stop exec failed: {}", e);
                    println!("    systemctl stop: skipped (not running)");
                }
            }

            dlog!("uninstall", "systemctl --user disable cokacdir");
            match Command::new("systemctl").args(["--user", "disable", "cokacdir"]).output() {
                Ok(out) => {
                    crate::core::debug::log_output("uninstall", "systemctl --user disable cokacdir", &out);
                    if out.status.success() {
                        println!("    systemctl disable: OK");
                    } else {
                        println!("    systemctl disable: skipped (not enabled)");
                    }
                }
                Err(e) => {
                    dlog!("uninstall", "systemctl disable exec failed: {}", e);
                    println!("    systemctl disable: skipped (not enabled)");
                }
            }

            // Kill any externally running cokacdir processes
            dlog!("uninstall", "Killing all cokacdir processes via pkill...");
            match Command::new("pkill").arg("cokacdir").output() {
                Ok(out) => {
                    crate::core::debug::log_output("uninstall", "pkill cokacdir", &out);
                    dlog!("uninstall", "pkill cokacdir exit={}", out.status.code().unwrap_or(-1));
                }
                Err(e) => {
                    dlog!("uninstall", "pkill cokacdir failed: {}", e);
                }
            }
        }
        Os::Windows => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let mut ps = Command::new("powershell");
                ps.args(["-NoProfile", "-NonInteractive", "-Command",
                    "Stop-ScheduledTask -TaskName 'cokacdir' -ErrorAction SilentlyContinue"]);
                ps.creation_flags(0x08000000);
                match ps.output() {
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        dlog!("uninstall", "Stop-ScheduledTask exit={}, stderr='{}'", out.status, stderr.trim());
                    }
                    Err(e) => {
                        dlog!("uninstall", "Stop-ScheduledTask failed: {}", e);
                    }
                }

                let mut cmd = Command::new("schtasks");
                cmd.args(["/Delete", "/TN", "cokacdir", "/F"]);
                cmd.creation_flags(0x08000000);
                match cmd.output() {
                    Ok(out) => {
                        crate::core::debug::log_output("uninstall", "schtasks /Delete /TN cokacdir /F", &out);
                        if out.status.success() {
                            println!("    schtasks delete: OK");
                        } else {
                            println!("    schtasks delete: skipped (not registered)");
                        }
                    }
                    Err(e) => {
                        dlog!("uninstall", "schtasks delete exec failed: {}", e);
                        println!("    schtasks delete: skipped (not registered)");
                    }
                }

                dlog!("uninstall", "Killing all cokacdir* processes via PowerShell...");
                let mut ps_kill = Command::new("powershell");
                ps_kill.args(["-NoProfile", "-NonInteractive", "-Command",
                    "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } | ForEach-Object { Write-Output \"Killing PID=$($_.Id) Name=$($_.ProcessName)\"; Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }"]);
                ps_kill.creation_flags(0x08000000);
                match ps_kill.output() {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        dlog!("uninstall", "kill cokacdir* exit={}, stdout='{}', stderr='{}'",
                            out.status, stdout.trim(), stderr.trim());
                    }
                    Err(e) => {
                        dlog!("uninstall", "kill cokacdir* failed: {}", e);
                    }
                }
            }
            #[cfg(not(windows))]
            {
                println!("    Windows service cleanup: skipped (not on Windows)");
            }
        }
    }

    println!();
    println!("  Removing files...");

    // Phase 2: Remove files and directories (platform-specific)
    for path in &files {
        if path.exists() {
            match std::fs::remove_file(path) {
                Ok(_) => {
                    dlog!("uninstall", "Removed: {}", path.display());
                    println!("    rm {}  ...OK", path.display());
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
                    && os != Os::Windows =>
                {
                    // /usr/local/bin/cokacdir typically requires root to remove;
                    // try sudo as a fallback so uninstall actually succeeds.
                    dlog!("uninstall", "Permission denied for {}, retrying with sudo", path.display());
                    let label = format!("sudo rm -f {}", path.display());
                    let status = Command::new("sudo")
                        .args(["rm", "-f", &path.to_string_lossy()])
                        .status();
                    match status {
                        Ok(s) => {
                            crate::core::debug::log_status("uninstall", &label, &s);
                            if s.success() {
                                dlog!("uninstall", "Removed via sudo: {}", path.display());
                                println!("    rm {} (sudo)  ...OK", path.display());
                            } else {
                                dlog!("uninstall", "sudo rm exit {:?}: {}", s.code(), path.display());
                                println!(
                                    "    rm {} (sudo)  ...failed (exit {:?})",
                                    path.display(),
                                    s.code()
                                );
                            }
                        }
                        Err(se) => {
                            dlog!("uninstall", "sudo rm could not run: {}", se);
                            println!("    rm {}  ...failed ({}, sudo unavailable: {})", path.display(), e, se);
                        }
                    }
                }
                Err(e) => {
                    dlog!("uninstall", "Failed: {} ({})", path.display(), e);
                    println!("    rm {}  ...failed ({})", path.display(), e);
                }
            }
        }
    }

    for path in &dirs {
        if path.exists() {
            match std::fs::remove_dir_all(path) {
                Ok(_) => {
                    dlog!("uninstall", "Removed dir: {}", path.display());
                    println!("    rm -rf {}  ...OK", path.display());
                }
                Err(e) => {
                    dlog!("uninstall", "Failed dir: {} ({})", path.display(), e);
                    println!("    rm -rf {}  ...failed ({})", path.display(), e);
                }
            }
        }
    }

    // Phase 3: Reload systemd to clear stale unit cache
    if os == Os::Linux {
        dlog!("uninstall", "systemctl --user daemon-reload");
        match Command::new("systemctl").args(["--user", "daemon-reload"]).output() {
            Ok(out) => crate::core::debug::log_output("uninstall", "systemctl --user daemon-reload", &out),
            Err(e) => dlog!("uninstall", "daemon-reload exec failed: {}", e),
        }
    }

    println!();
    println!("  Uninstall complete.");
    Ok(())
}

fn collect_paths(home: &PathBuf, os: Os) -> (Vec<PathBuf>, Vec<PathBuf>) {
    match os {
        Os::MacOS | Os::Linux => {
            let mut dirs = vec![
                home.join("Library/Logs/cokacdir"),
                home.join(".local/state/cokacdir"),
            ];
            if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
                let xdg_dir = PathBuf::from(xdg).join("cokacdir");
                if !dirs.contains(&xdg_dir) {
                    dirs.push(xdg_dir);
                }
            }
            (
                vec![
                    home.join(".local/bin/cokacdir"),
                    PathBuf::from("/usr/local/bin/cokacdir"),
                    home.join("Library/LaunchAgents/com.cokacdir.server.plist"),
                    home.join(".local/log/cokacdir.log"),
                    home.join(".config/systemd/user/cokacdir.service"),
                ],
                dirs,
            )
        }
        Os::Windows => (
            vec![
                home.join("cokacdir.exe"),
                home.join(".cokacdir/windows-service.json"),
            ],
            vec![
                home.join(".cokacdir/logs"),
                home.join(".cokacdir/scripts"),
            ],
        ),
    }
}
