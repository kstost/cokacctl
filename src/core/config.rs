use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILENAME: &str = "cokacctl.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Telegram bot tokens.
    #[serde(default)]
    pub tokens: Vec<String>,
    /// Disabled token values (subset of tokens).
    #[serde(default)]
    pub disabled_tokens: Vec<String>,
    /// Path to the cokacdir binary (if custom).
    #[serde(default)]
    pub install_path: Option<String>,
}

impl Config {
    /// Returns only the tokens that are not disabled.
    pub fn active_tokens(&self) -> Vec<String> {
        self.tokens.iter()
            .filter(|t| !self.disabled_tokens.contains(t))
            .cloned()
            .collect()
    }

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
        match serde_json::from_str::<Config>(&content) {
            Ok(config) => {
                dlog!("config", "Config loaded: {} tokens", config.tokens.len());
                config
            }
            Err(e) => {
                // Back up the corrupt file before falling back to defaults so
                // that the user's tokens aren't silently overwritten on next save.
                let backup = path.with_extension("json.bak");
                let _ = std::fs::remove_file(&backup);
                let renamed = std::fs::rename(&path, &backup).is_ok();
                dlog!(
                    "config",
                    "Config parse failed ({}), backed up to {}: {}",
                    e,
                    backup.display(),
                    renamed
                );
                eprintln!(
                    "Warning: Config file corrupt ({}). Previous contents backed up to {}",
                    e,
                    backup.display()
                );
                Config::default()
            }
        }
    }

    /// Save config to disk with restricted permissions.
    ///
    /// On Unix, uses write-to-tmp + rename so the file is never visible at
    /// default permissions (0644) — tokens would otherwise be briefly readable
    /// by other users between `write` and `set_permissions`.
    pub fn save(&self) -> Result<(), String> {
        dlog!("config", "Saving config ({} tokens, {} disabled)...", self.tokens.len(), self.disabled_tokens.len());
        let path = Self::path();
        if let Some(parent) = path.parent() {
            dlog!("config", "Ensuring config dir exists: {}", parent.display());
            std::fs::create_dir_all(parent)
                .map_err(|e| {
                    dlog!("config", "create_dir_all failed: {}", e);
                    format!("Cannot create config dir: {}", e)
                })?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("JSON serialize failed: {}", e))?;
        dlog!("config", "Serialized config: {} bytes", content.len());

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp = path.with_extension("json.tmp");
            match std::fs::remove_file(&tmp) {
                Ok(_) => dlog!("config", "Cleared stale tmp: {}", tmp.display()),
                Err(e) => dlog!("config", "Tmp cleanup: {} (ok if nonexistent)", e),
            }
            {
                dlog!("config", "Opening tmp with mode 0o600: {}", tmp.display());
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&tmp)
                    .map_err(|e| {
                        dlog!("config", "tmp open failed: {}", e);
                        format!("Cannot create config temp: {}", e)
                    })?;
                file.write_all(content.as_bytes())
                    .map_err(|e| {
                        dlog!("config", "tmp write_all failed: {}", e);
                        format!("Cannot write config: {}", e)
                    })?;
                dlog!("config", "Wrote {} bytes to tmp", content.len());
                match file.sync_all() {
                    Ok(_) => dlog!("config", "fsync OK"),
                    Err(e) => dlog!("config", "fsync failed (non-fatal): {}", e),
                }
            }
            dlog!("config", "Renaming tmp -> {}", path.display());
            std::fs::rename(&tmp, &path)
                .map_err(|e| {
                    dlog!("config", "rename tmp->path failed: {}", e);
                    format!("Cannot finalize config: {}", e)
                })?;
        }
        #[cfg(not(unix))]
        {
            dlog!("config", "Writing (non-unix): {}", path.display());
            std::fs::write(&path, &content)
                .map_err(|e| {
                    dlog!("config", "write failed: {}", e);
                    format!("Cannot write config: {}", e)
                })?;
        }

        dlog!("config", "Config saved to {}", path.display());
        Ok(())
    }
}
