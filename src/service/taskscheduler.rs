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
}

impl ServiceManager for TaskSchedulerManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        dlog!("taskscheduler", "start() called - binary: {}, tokens: {}", binary_path.display(), tokens.len());

        // Remove existing task first
        dlog!("taskscheduler", "Removing existing task...");
        let _ = self.remove();

        // Build the command: "path\to\binary" --ccserver -- token1 token2
        let token_args = tokens.join(" ");
        let tr = format!(
            "\"{}\" --ccserver -- {}",
            binary_path.to_string_lossy(),
            token_args
        );
        dlog!("taskscheduler", "Task command: {}", tr);

        // Create task with schtasks
        dlog!("taskscheduler", "Creating task with schtasks /Create...");
        let output = Command::new("schtasks")
            .args([
                "/Create",
                "/TN", TASK_NAME,
                "/TR", &tr,
                "/SC", "ONLOGON",
                "/RL", "LIMITED",
                "/F",
            ])
            .output()
            .map_err(|e| {
                dlog!("taskscheduler", "schtasks /Create spawn failed: {}", e);
                format!("Failed to run schtasks: {}", e)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        dlog!("taskscheduler", "schtasks /Create exit: {}, stdout: '{}', stderr: '{}'",
            output.status, stdout.trim(), stderr.trim());

        if !output.status.success() {
            return Err(format!("Task creation failed: {}", stderr.trim()));
        }

        // Start the task immediately
        dlog!("taskscheduler", "Starting task with schtasks /Run...");
        let run_output = Command::new("schtasks")
            .args(["/Run", "/TN", TASK_NAME])
            .output()
            .map_err(|e| {
                dlog!("taskscheduler", "schtasks /Run spawn failed: {}", e);
                format!("Failed to start task: {}", e)
            })?;

        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        let run_stderr = String::from_utf8_lossy(&run_output.stderr);
        dlog!("taskscheduler", "schtasks /Run exit: {}, stdout: '{}', stderr: '{}'",
            run_output.status, run_stdout.trim(), run_stderr.trim());

        if !run_output.status.success() {
            return Err(format!("Task start failed: {}", run_stderr.trim()));
        }

        dlog!("taskscheduler", "start() completed successfully");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("taskscheduler", "stop() called");
        let output = Command::new("schtasks")
            .args(["/End", "/TN", TASK_NAME])
            .output()
            .map_err(|e| {
                dlog!("taskscheduler", "schtasks /End failed: {}", e);
                format!("Failed to stop task: {}", e)
            })?;

        dlog!("taskscheduler", "schtasks /End exit: {}", output.status);
        if !output.status.success() {
            dlog!("taskscheduler", "stop() task may not have been running");
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("taskscheduler", "remove() called");
        let output = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output();
        match &output {
            Ok(o) => dlog!("taskscheduler", "schtasks /Delete exit: {}", o.status),
            Err(e) => dlog!("taskscheduler", "schtasks /Delete failed: {}", e),
        }
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("taskscheduler", "status() called");
        match Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    dlog!("taskscheduler", "status(): task not found");
                    return ServiceStatus::NotInstalled;
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let line = stdout.trim();
                dlog!("taskscheduler", "status() raw output: '{}'", line);
                if line.contains("Running") {
                    dlog!("taskscheduler", "status(): Running");
                    ServiceStatus::Running
                } else if line.contains("Ready") || line.contains("Disabled") {
                    dlog!("taskscheduler", "status(): Stopped");
                    ServiceStatus::Stopped
                } else if line.is_empty() {
                    dlog!("taskscheduler", "status(): NotInstalled (empty)");
                    ServiceStatus::NotInstalled
                } else {
                    dlog!("taskscheduler", "status(): Stopped (unknown: {})", line);
                    ServiceStatus::Stopped
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "status() query failed: {}", e);
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
