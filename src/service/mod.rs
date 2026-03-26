pub mod launchd;
pub mod systemd;
pub mod taskscheduler;

use std::path::Path;

/// Service status.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    NotInstalled,
    Unknown(String),
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::Running => write!(f, "Running"),
            ServiceStatus::Stopped => write!(f, "Stopped"),
            ServiceStatus::NotInstalled => write!(f, "Not installed"),
            ServiceStatus::Unknown(s) => write!(f, "Unknown ({})", s),
        }
    }
}

/// Common interface for OS-specific service managers.
pub trait ServiceManager {
    /// Register and start the service with given tokens.
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String>;
    /// Stop the service.
    fn stop(&self) -> Result<(), String>;
    /// Restart the service (stop + start with existing config).
    fn restart(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        self.stop().ok(); // may already be stopped
        self.start(binary_path, tokens)
    }
    /// Remove the service entirely.
    fn remove(&self) -> Result<(), String>;
    /// Get current service status.
    fn status(&self) -> ServiceStatus;
    /// Get log file path.
    fn log_path(&self) -> Option<std::path::PathBuf>;
}

/// Get the appropriate ServiceManager for the current OS.
pub fn manager() -> Box<dyn ServiceManager> {
    match crate::core::platform::Os::detect() {
        crate::core::platform::Os::MacOS => Box::new(launchd::LaunchdManager::new()),
        crate::core::platform::Os::Linux => Box::new(systemd::SystemdManager::new()),
        crate::core::platform::Os::Windows => {
            Box::new(taskscheduler::TaskSchedulerManager::new())
        }
    }
}
