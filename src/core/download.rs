use std::path::Path;
use super::{ProgressMsg, ProgressTx};

fn send(tx: &Option<ProgressTx>, msg: String) {
    if let Some(tx) = tx {
        tx.send(ProgressMsg::Log(msg)).ok();
    } else {
        println!("{}", msg);
    }
}

/// Download a file from `url` to `dest`.
pub async fn download_file(url: &str, dest: &Path, tx: &Option<ProgressTx>) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }

    let total = resp.content_length();
    let stream = resp.bytes().await.map_err(|e| format!("Read failed: {}", e))?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create directory {}: {}", parent.display(), e))?;
    }

    std::fs::write(dest, &stream).map_err(|e| format!("Write failed: {}", e))?;

    if let Some(total) = total {
        send(tx, format!("  Downloaded {:.1} MB", total as f64 / 1_048_576.0));
    }

    Ok(())
}

/// Download with a temporary file, then move into place.
pub async fn download_to_path(url: &str, dest: &Path, tx: &Option<ProgressTx>) -> Result<(), String> {
    let tmp = dest.with_extension("tmp");
    download_file(url, &tmp, tx).await?;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod failed: {}", e))?;
    }

    // Move into place
    if dest.exists() {
        // Try removing directly first
        if std::fs::remove_file(dest).is_err() {
            // File may be locked (running on Windows) — rename dance
            let old = dest.with_extension("old");
            std::fs::remove_file(&old).ok(); // clean up previous .old if any
            if std::fs::rename(dest, &old).is_ok() {
                // Will be cleaned up later or on next run
            } else {
                return Err(format!(
                    "Cannot replace {}: file may be in use",
                    dest.display()
                ));
            }
        }
    }
    std::fs::rename(&tmp, dest).or_else(|_| -> Result<(), String> {
        std::fs::copy(&tmp, dest)
            .map_err(|e| format!("Copy failed: {}", e))?;
        std::fs::remove_file(&tmp).ok();
        Ok(())
    })?;

    // Clean up old binary if rename dance was used
    let old = dest.with_extension("old");
    std::fs::remove_file(&old).ok();

    Ok(())
}
