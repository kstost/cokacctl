use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Os {
    MacOS,
    Linux,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Os {
    pub fn detect() -> Self {
        let os = match std::env::consts::OS {
            "macos" => Os::MacOS,
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            other => {
                eprintln!("Unsupported OS: {}", other);
                std::process::exit(1);
            }
        };
        dlog!("platform", "Detected OS: {:?}", os);
        os
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Os::MacOS => "macos",
            Os::Linux => "linux",
            Os::Windows => "windows",
        }
    }
}

impl Arch {
    pub fn detect() -> Self {
        let arch = match std::env::consts::ARCH {
            "x86_64" | "amd64" => Arch::X86_64,
            "aarch64" | "arm64" => Arch::Aarch64,
            other => {
                eprintln!("Unsupported architecture: {}", other);
                std::process::exit(1);
            }
        };
        dlog!("platform", "Detected Arch: {:?}", arch);
        arch
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }
}

/// URL to download the cokacdir binary for the current platform.
pub fn binary_download_url(os: Os, arch: Arch) -> String {
    let ext = if os == Os::Windows { ".exe" } else { "" };
    let url = format!(
        "https://cokacdir.cokac.com/dist/cokacdir-{}-{}{}",
        os.as_str(),
        arch.as_str(),
        ext
    );
    dlog!("platform", "Binary download URL: {}", url);
    url
}

/// Default installation path for cokacdir binary.
pub fn default_install_path(os: Os) -> PathBuf {
    let path = match os {
        Os::Windows => {
            let home = dirs::home_dir().expect("Cannot determine home directory");
            home.join("cokacdir.exe")
        }
        _ => PathBuf::from("/usr/local/bin/cokacdir"),
    };
    dlog!("platform", "Default install path: {}", path.display());
    path
}

/// Fallback installation path when default is not writable.
pub fn fallback_install_path() -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    let dir = home.join(".local").join("bin");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join("cokacdir");
    dlog!("platform", "Fallback install path: {}", path.display());
    path
}

/// Find cokacdir binary in PATH or default install location.
pub fn find_cokacdir() -> Option<PathBuf> {
    dlog!("platform", "Searching for cokacdir...");
    if let Some(p) = which("cokacdir") {
        dlog!("platform", "Found cokacdir in PATH: {}", p.display());
        return Some(p);
    }
    // Fallback: check default install path
    let default = default_install_path(Os::detect());
    if default.is_file() {
        dlog!("platform", "Found cokacdir at default path: {}", default.display());
        return Some(default);
    }
    let fallback = fallback_install_path();
    if fallback.is_file() {
        dlog!("platform", "Found cokacdir at fallback path: {}", fallback.display());
        return Some(fallback);
    }
    dlog!("platform", "cokacdir not found anywhere");
    None
}

/// Simple which implementation.
pub fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    dlog!("platform::which", "Searching PATH for '{}'", name);
    for dir in path_var.split(sep) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            dlog!("platform::which", "Found: {}", candidate.display());
            return Some(candidate);
        }
        // Windows: try with .exe
        if cfg!(windows) {
            let exe = PathBuf::from(dir).join(format!("{}.exe", name));
            if exe.is_file() {
                dlog!("platform::which", "Found (exe): {}", exe.display());
                return Some(exe);
            }
        }
    }
    dlog!("platform::which", "'{}' not found in PATH", name);
    None
}

/// Service-related paths per platform.
pub struct ServicePaths {
    pub service_file: PathBuf,
    pub wrapper_script: PathBuf,
    pub log_dir: PathBuf,
    pub log_file: PathBuf,
    pub error_log_file: PathBuf,
}

impl ServicePaths {
    pub fn for_current_os() -> Self {
        let home = dirs::home_dir().expect("Cannot determine home directory");
        let paths = match Os::detect() {
            Os::MacOS => {
                let log_dir = home.join("Library/Logs/cokacdir");
                ServicePaths {
                    service_file: home.join("Library/LaunchAgents/com.cokacdir.server.plist"),
                    wrapper_script: log_dir.join("run.sh"),
                    log_dir: log_dir.clone(),
                    log_file: log_dir.join("cokacdir.log"),
                    error_log_file: log_dir.join("cokacdir.error.log"),
                }
            }
            Os::Linux => {
                let state_dir = std::env::var("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| home.join(".local/state"));
                let log_dir = state_dir.join("cokacdir");
                ServicePaths {
                    service_file: home.join(".config/systemd/user/cokacdir.service"),
                    wrapper_script: log_dir.join("run.sh"),
                    log_dir: log_dir.clone(),
                    log_file: log_dir.join("cokacdir.log"),
                    error_log_file: log_dir.join("cokacdir.error.log"),
                }
            }
            Os::Windows => {
                let log_dir = home.join(".cokacdir").join("logs");
                ServicePaths {
                    service_file: PathBuf::new(), // Task Scheduler has no file
                    wrapper_script: PathBuf::new(),
                    log_dir: log_dir.clone(),
                    log_file: log_dir.join("cokacdir.log"),
                    error_log_file: log_dir.join("cokacdir.error.log"),
                }
            }
        };
        dlog!("platform", "Service paths - log_file: {}", paths.log_file.display());
        paths
    }
}

/// Get shell config file path.
pub fn shell_config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let shell = std::env::var("SHELL").unwrap_or_default();
    let path = if shell.ends_with("zsh") {
        Some(home.join(".zshrc"))
    } else if shell.ends_with("bash") {
        let bashrc = home.join(".bashrc");
        let profile = home.join(".bash_profile");
        if bashrc.exists() {
            Some(bashrc)
        } else if profile.exists() {
            Some(profile)
        } else {
            Some(bashrc)
        }
    } else {
        None
    };
    dlog!("platform", "Shell config path: {:?}", path);
    path
}
