use super::{ServiceManager, ServiceStatus};
use crate::core::debug::decode_output;
use std::path::{Path, PathBuf};
use std::process::Command;

const TASK_NAME: &str = "cokacdir";

pub struct TaskSchedulerManager;

impl TaskSchedulerManager {
    pub fn new() -> Self {
        dlog!("taskscheduler", "TaskSchedulerManager created");
        TaskSchedulerManager
    }

    fn powershell(script: &str) -> Result<std::process::Output, String> {
        dlog!("taskscheduler", "PowerShell: {}", script);
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .map_err(|e| format!("PowerShell execution failed: {}", e))
    }
}

impl ServiceManager for TaskSchedulerManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        dlog!("taskscheduler", "========== start() BEGIN ==========");
        dlog!("taskscheduler", "binary_path: '{}'", binary_path.display());
        dlog!("taskscheduler", "binary_path exists: {}", binary_path.exists());
        dlog!("taskscheduler", "tokens count: {}", tokens.len());

        // Remove existing task first
        dlog!("taskscheduler", "[step 1/4] Removing existing task...");
        let remove_result = self.remove();
        dlog!("taskscheduler", "remove result: {:?}", remove_result);

        // PowerShell single-quoted strings: only ' needs escaping as ''
        // Inside '...', $, `, ; are all literal — no injection possible
        let escape_ps_single = |s: &str| -> String {
            s.replace('\'', "''")
        };

        let token_args = tokens.join(" ");
        let args_str = format!("--ccserver -- {}", token_args);
        let binary = escape_ps_single(&binary_path.to_string_lossy());
        let args = escape_ps_single(&args_str);
        let home = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
        let home = escape_ps_single(&home);

        dlog!("taskscheduler", "[step 2/4] Building PowerShell script");
        dlog!("taskscheduler", "  binary: '{}'", binary);
        dlog!("taskscheduler", "  working_dir: '{}'", home);
        dlog!("taskscheduler", "  token count in args: {}", tokens.len());

        let script = format!(
            "$action = New-ScheduledTaskAction -Execute '{binary}' -Argument '{args}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -RunLevel Highest -Force",
            binary = binary,
            args = args,
            wd = home,
            name = TASK_NAME,
        );

        dlog!("taskscheduler", "[step 2/4] Script built (tokens redacted)");
        dlog!("taskscheduler", "[step 3/4] Executing Register-ScheduledTask...");

        let t0 = std::time::Instant::now();
        let output = Self::powershell(&script)?;
        let elapsed = t0.elapsed();

        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "Register took: {:?}", elapsed);
        dlog!("taskscheduler", "Register exit code: {}", output.status);
        dlog!("taskscheduler", "Register stdout: '{}'", stdout.trim());
        dlog!("taskscheduler", "Register stderr: '{}'", stderr.trim());

        if !output.status.success() {
            dlog!("taskscheduler", "========== start() FAILED at Register ==========");
            return Err(format!("Task creation failed: {}", stderr.trim()));
        }

        // Start the task immediately
        dlog!("taskscheduler", "[step 4/4] Executing Start-ScheduledTask...");
        let start_script = format!("Start-ScheduledTask -TaskName '{}'", TASK_NAME);
        dlog!("taskscheduler", "Start script: {}", start_script);

        let t1 = std::time::Instant::now();
        let start_output = Self::powershell(&start_script)?;
        let start_elapsed = t1.elapsed();

        let start_stdout = decode_output(&start_output.stdout);
        let start_stderr = decode_output(&start_output.stderr);
        dlog!("taskscheduler", "Start took: {:?}", start_elapsed);
        dlog!("taskscheduler", "Start exit code: {}", start_output.status);
        dlog!("taskscheduler", "Start stdout: '{}'", start_stdout.trim());
        dlog!("taskscheduler", "Start stderr: '{}'", start_stderr.trim());

        if !start_output.status.success() {
            dlog!("taskscheduler", "========== start() FAILED at Start ==========");
            return Err(format!("Task start failed: {}", start_stderr.trim()));
        }

        // Wait briefly then check if the process is actually running
        std::thread::sleep(std::time::Duration::from_millis(1500));
        dlog!("taskscheduler", "Checking if cokacdir.exe is running after start...");
        let check = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq cokacdir.exe", "/FO", "CSV", "/NH"])
            .output();
        if let Ok(ref out) = check {
            let stdout = decode_output(&out.stdout);
            dlog!("taskscheduler", "Post-start tasklist: '{}'", stdout.trim());
            if !stdout.contains("cokacdir.exe") {
                dlog!("taskscheduler", "WARNING: cokacdir.exe not found after Start-ScheduledTask!");
                dlog!("taskscheduler", "Trying direct process spawn as fallback...");
                // Fallback: spawn cokacdir directly as a detached process
                let child = Command::new(binary_path)
                    .args(["--ccserver", "--"])
                    .args(tokens)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                match child {
                    Ok(c) => dlog!("taskscheduler", "Direct spawn OK, pid: {}", c.id()),
                    Err(e) => {
                        dlog!("taskscheduler", "Direct spawn failed: {}", e);
                        return Err(format!("Failed to start cokacdir: {}", e));
                    }
                }
            }
        }

        dlog!("taskscheduler", "========== start() SUCCESS ==========");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("taskscheduler", "stop() called");

        // Stop the scheduled task
        let stop_result = Self::powershell(&format!(
            "Stop-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue", TASK_NAME
        ));
        if let Ok(ref out) = stop_result {
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "Stop-ScheduledTask exit: {}, stderr: '{}'", out.status, stderr.trim());
        }

        // Also kill any running cokacdir process
        let kill_result = Command::new("taskkill")
            .args(["/IM", "cokacdir.exe", "/F"])
            .output();
        if let Ok(ref out) = kill_result {
            let stdout = decode_output(&out.stdout);
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "taskkill exit: {}, stdout: '{}', stderr: '{}'",
                out.status, stdout.trim(), stderr.trim());
        }
        dlog!("taskscheduler", "stop() done");

        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("taskscheduler", "remove() called");

        // Stop first
        let _ = self.stop();

        // Delete the scheduled task
        let del_result = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output();
        if let Ok(ref out) = del_result {
            let stdout = decode_output(&out.stdout);
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "schtasks /Delete exit: {}, stdout: '{}', stderr: '{}'",
                out.status, stdout.trim(), stderr.trim());
        }
        dlog!("taskscheduler", "remove() done");

        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("taskscheduler", "status() called");

        // Check if cokacdir.exe process is actually running
        match Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq cokacdir.exe", "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(output) => {
                let stdout = decode_output(&output.stdout);
                let line = stdout.trim();
                dlog!("taskscheduler", "status() tasklist: '{}'", line);
                if line.contains("cokacdir.exe") {
                    dlog!("taskscheduler", "status(): Running");
                    return ServiceStatus::Running;
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "status() tasklist failed: {}", e);
            }
        }

        // Check if the scheduled task exists
        match Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(output) => {
                let stdout = decode_output(&output.stdout);
                let stderr = decode_output(&output.stderr);
                if !output.status.success() {
                    dlog!("taskscheduler", "status(): NotInstalled (stderr: '{}')", stderr.trim());
                    ServiceStatus::NotInstalled
                } else {
                    dlog!("taskscheduler", "status(): Stopped (stdout: '{}')", stdout.trim());
                    ServiceStatus::Stopped
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "status() schtasks failed: {}", e);
                ServiceStatus::Unknown("Cannot query Task Scheduler".into())
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let path = home.join(".cokacdir").join("logs").join("cokacdir.log");
        dlog!("taskscheduler", "log_path: {}", path.display());
        Some(path)
    }
}
