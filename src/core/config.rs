use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILENAME: &str = "cokacctl.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Telegram bot tokens.
    #[serde(default)]
    pub tokens: Vec<String>,
    /// Path to the cokacdir binary (if custom).
    #[serde(default)]
    pub install_path: Option<String>,
}

impl Config {
    /// Config file path: ~/.cokacdir/cokacctl.json
    pub fn path() -> PathBuf {
        let home = dirs::home_dir().expect("Cannot determine home directory");
        home.join(".cokacdir").join(CONFIG_FILENAME)
    }

    /// Load config from disk. Returns default if file doesn't exist.
    pub fn load() -> Self {
        let path = Self::path();
        if !path.exists() {
            return Config::default();
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Config::default(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Save config to disk with restricted permissions.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create config dir: {}", e))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;
        std::fs::write(&path, &content)
            .map_err(|e| format!("Cannot write config: {}", e))?;

        // Restrict permissions on Unix (0o600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
        }

        Ok(())
    }
}
