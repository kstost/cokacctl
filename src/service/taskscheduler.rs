use super::{ServiceManager, ServiceStatus};
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
        dlog!("taskscheduler", "start() called - binary: {}, tokens: {}", binary_path.display(), tokens.len());

        // Remove existing task first
        dlog!("taskscheduler", "Removing existing task...");
        let _ = self.remove();

        let token_args = tokens.join(" ");
        let args_str = format!("--ccserver -- {}", token_args);
        let binary = binary_path.to_string_lossy().replace('\'', "''");
        let args = args_str.replace('\'', "''");
        let home = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default()
            .replace('\'', "''");

        let script = format!(
            "$action = New-ScheduledTaskAction -Execute '{binary}' -Argument '{args}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -RunLevel Highest -Force",
            binary = binary,
            args = args,
            wd = home,
            name = TASK_NAME,
        );

        dlog!("taskscheduler", "Creating scheduled task via PowerShell...");
        let output = Self::powershell(&script)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        dlog!("taskscheduler", "Register exit: {}, stdout: '{}', stderr: '{}'",
            output.status, stdout.trim(), stderr.trim());

        if !output.status.success() {
            return Err(format!("Task creation failed: {}", stderr.trim()));
        }

        // Start the task immediately
        dlog!("taskscheduler", "Starting task...");
        let start_script = format!("Start-ScheduledTask -TaskName '{}'", TASK_NAME);
        let start_output = Self::powershell(&start_script)?;

        let start_stderr = String::from_utf8_lossy(&start_output.stderr);
        dlog!("taskscheduler", "Start exit: {}, stderr: '{}'", start_output.status, start_stderr.trim());

        if !start_output.status.success() {
            return Err(format!("Task start failed: {}", start_stderr.trim()));
        }

        dlog!("taskscheduler", "start() completed successfully");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("taskscheduler", "stop() called");

        // Stop the scheduled task
        let _ = Self::powershell(&format!(
            "Stop-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue", TASK_NAME
        ));

        // Also kill any running cokacdir process
        let _ = Command::new("taskkill")
            .args(["/IM", "cokacdir.exe", "/F"])
            .output();
        dlog!("taskscheduler", "stop() done");

        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("taskscheduler", "remove() called");

        // Stop first
        let _ = self.stop();

        // Delete the scheduled task
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output();
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
                let stdout = String::from_utf8_lossy(&output.stdout);
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
                if !output.status.success() {
                    dlog!("taskscheduler", "status(): NotInstalled");
                    ServiceStatus::NotInstalled
                } else {
                    dlog!("taskscheduler", "status(): Stopped");
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
