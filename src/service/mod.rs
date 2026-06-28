pub mod launchd;
pub mod systemd;
pub mod taskscheduler;

use std::path::Path;

/// Service status.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    DirectRunning(String),
    DirectStopped(String),
    NotInstalled,
    Unknown(String),
}

impl ServiceStatus {
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            ServiceStatus::Running | ServiceStatus::DirectRunning(_)
        )
    }

    pub fn service_registration_unavailable(&self) -> bool {
        matches!(
            self,
            ServiceStatus::DirectRunning(_) | ServiceStatus::DirectStopped(_)
        )
    }
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::Running => write!(f, "Running"),
            ServiceStatus::Stopped => write!(f, "Stopped"),
            ServiceStatus::DirectRunning(s) => write!(f, "Running direct ({})", s),
            ServiceStatus::DirectStopped(s) => write!(f, "Stopped direct ({})", s),
            ServiceStatus::NotInstalled => write!(f, "Not installed"),
            ServiceStatus::Unknown(s) => write!(f, "Unknown ({})", s),
        }
    }
}

/// Common interface for OS-specific service managers.
pub trait ServiceManager: Send + Sync {
    /// Register and start the service with given tokens.
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String>;
    /// Stop the service.
    fn stop(&self) -> Result<(), String>;
    /// Restart the service (stop + start with existing config).
    fn restart(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        dlog!(
            "service",
            "restart: stop + start (bin={}, tokens={})",
            binary_path.display(),
            tokens.len()
        );
        match self.stop() {
            Ok(_)  => dlog!("service", "restart: stop ok"),
            Err(e) => dlog!("service", "restart: stop returned err (continuing): {}", e),
        }
        let r = self.start(binary_path, tokens);
        dlog!("service", "restart: start result is_ok={}", r.is_ok());
        r
    }
    /// Remove the service entirely.
    fn remove(&self) -> Result<(), String>;
    /// Get current service status.
    fn status(&self) -> ServiceStatus;
    /// Check if any cokacdir process is running externally (regardless of service manager).
    fn is_any_running(&self) -> bool;
    /// Get log file path.
    fn log_path(&self) -> Option<std::path::PathBuf>;
}

/// Get the appropriate ServiceManager for the current OS.
pub fn manager() -> Box<dyn ServiceManager + Send + Sync> {
    match crate::core::platform::Os::detect() {
        crate::core::platform::Os::MacOS => Box::new(launchd::LaunchdManager::new()),
        crate::core::platform::Os::Linux => Box::new(systemd::SystemdManager::new()),
        crate::core::platform::Os::Windows => Box::new(taskscheduler::TaskSchedulerManager::new()),
    }
}
