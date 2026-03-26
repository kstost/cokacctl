use super::{ServiceManager, ServiceStatus};
use std::path::{Path, PathBuf};
use std::process::Command;

const TASK_NAME: &str = "cokacdir";

pub struct TaskSchedulerManager;

impl TaskSchedulerManager {
    pub fn new() -> Self {
        TaskSchedulerManager
    }

    fn powershell(script: &str) -> Result<std::process::Output, String> {
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .map_err(|e| format!("PowerShell execution failed: {}", e))
    }

    fn escape_ps_string(s: &str) -> String {
        s.replace('\'', "''")
    }
}

impl ServiceManager for TaskSchedulerManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        // Build token argument string
        let token_args: Vec<String> = tokens.iter().map(|t| t.clone()).collect();
        let args_str = format!("--ccserver -- {}", token_args.join(" "));
        let binary = Self::escape_ps_string(&binary_path.to_string_lossy());
        let args = Self::escape_ps_string(&args_str);
        let home = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
        let working_dir = Self::escape_ps_string(&home);

        // Remove existing task first
        let _ = self.remove();

        let script = format!(
            "$action = New-ScheduledTaskAction -Execute '{binary}' -Argument '{args}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -RunLevel Limited -LogonType Interactive\n\
             $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Seconds 10) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Days 0)\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null\n\
             Start-ScheduledTask -TaskName '{name}'",
            binary = binary,
            args = args,
            wd = working_dir,
            name = TASK_NAME,
        );

        let output = Self::powershell(&script)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Access") || stderr.contains("denied") || stderr.contains("privilege") {
                return Err(format!(
                    "Task creation failed (access denied). A previous task may have been registered with admin privileges. \
                     Try running as administrator once, or manually delete the '{}' task in Task Scheduler.",
                    TASK_NAME
                ));
            }
            return Err(format!("Task creation failed: {}", stderr));
        }

        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let script = format!(
            "Stop-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue",
            TASK_NAME
        );
        let _ = Self::powershell(&script);
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        // Try PowerShell first
        let script = format!(
            "Unregister-ScheduledTask -TaskName '{}' -Confirm:$false -ErrorAction SilentlyContinue",
            TASK_NAME
        );
        let _ = Self::powershell(&script);

        // If task still exists (e.g. registered with elevated privileges), try schtasks
        let check = format!(
            "(Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue).TaskName",
            TASK_NAME
        );
        if let Ok(output) = Self::powershell(&check) {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                let _ = Command::new("schtasks")
                    .args(["/Delete", "/TN", TASK_NAME, "/F"])
                    .output();
            }
        }

        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        let script = format!(
            "(Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue).State",
            TASK_NAME
        );
        match Self::powershell(&script) {
            Ok(output) => {
                let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
                match state.as_str() {
                    "Running" => ServiceStatus::Running,
                    "Ready" => ServiceStatus::Stopped,
                    "" => ServiceStatus::NotInstalled,
                    _ => ServiceStatus::Unknown(state),
                }
            }
            Err(_) => ServiceStatus::Unknown("Cannot query Task Scheduler".into()),
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join(".cokacdir").join("logs").join("cokacdir.log"))
    }
}
