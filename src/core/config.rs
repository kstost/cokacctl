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
        let path = home.join(".cokacdir").join(CONFIG_FILENAME);
        dlog!("config", "Config path: {}", path.display());
        path
    }

    /// Load config from disk. Returns default if file doesn't exist.
    pub fn load() -> Self {
        let path = Self::path();
        if !path.exists() {
            dlog!("config", "Config file not found, using defaults");
            return Config::default();
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => {
                dlog!("config", "Config file read ({} bytes)", c.len());
                c
            }
            Err(e) => {
                dlog!("config", "Failed to read config: {}", e);
                return Config::default();
            }
        };
        let config: Config = serde_json::from_str(&content).unwrap_or_default();
        dlog!("config", "Config loaded: {} tokens", config.tokens.len());
        config
    }

    /// Save config to disk with restricted permissions.
    pub fn save(&self) -> Result<(), String> {
        dlog!("config", "Saving config ({} tokens)...", self.tokens.len());
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

        dlog!("config", "Config saved to {}", path.display());
        Ok(())
    }
}
