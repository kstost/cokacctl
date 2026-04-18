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
    dlog!("download", "Downloading {} -> {}", url, dest.display());
    let started = std::time::Instant::now();
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            dlog!("download", "Request failed: {}", e);
            format!("Download request failed: {}", e)
        })?;

    dlog!("download", "HTTP response status: {}", resp.status());
    dlog!("download", "HTTP response headers: {:?}", resp.headers());
    if !resp.status().is_success() {
        dlog!("download", "HTTP error: {}", resp.status());
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }

    let total = resp.content_length();
    dlog!("download", "Content length: {:?}", total);
    let stream = resp.bytes().await.map_err(|e| {
        dlog!("download", "Read failed: {}", e);
        format!("Read failed: {}", e)
    })?;
    dlog!("download", "Body received: {} bytes in {:?}", stream.len(), started.elapsed());

    if let Some(parent) = dest.parent() {
        dlog!("download", "Ensuring parent dir exists: {}", parent.display());
        std::fs::create_dir_all(parent)
            .map_err(|e| {
                dlog!("download", "create_dir_all failed: {}", e);
                format!("Cannot create directory {}: {}", parent.display(), e)
            })?;
    }

    dlog!("download", "Writing {} bytes to {}", stream.len(), dest.display());
    std::fs::write(dest, &stream).map_err(|e| {
        dlog!("download", "Write failed: {}", e);
        format!("Write failed: {}", e)
    })?;

    dlog!("download", "Downloaded {} bytes to {}", stream.len(), dest.display());
    if let Some(total) = total {
        send(tx, format!("  Downloaded {:.1} MB", total as f64 / 1_048_576.0));
    }

    Ok(())
}

/// Download with a temporary file, then move into place.
pub async fn download_to_path(url: &str, dest: &Path, tx: &Option<ProgressTx>) -> Result<(), String> {
    dlog!("download", "download_to_path: {} -> {}", url, dest.display());
    let tmp = dest.with_extension("tmp");
    download_file(url, &tmp, tx).await?;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod failed: {}", e))?;
        dlog!("download", "Set executable permission on {}", tmp.display());
    }

    // Move into place
    if dest.exists() {
        dlog!("download", "Destination exists, removing: {}", dest.display());
        match std::fs::remove_file(dest) {
            Ok(_) => dlog!("download", "Removed existing dest"),
            Err(e) => {
                dlog!("download", "Remove failed ({}), trying rename dance", e);
                let old = dest.with_extension("old");
                match std::fs::remove_file(&old) {
                    Ok(_) => dlog!("download", "Cleared stale .old"),
                    Err(e) => dlog!("download", "Stale .old cleanup: {} (ok if nonexistent)", e),
                }
                match std::fs::rename(dest, &old) {
                    Ok(_) => dlog!("download", "Renamed dest -> .old"),
                    Err(re) => {
                        dlog!("download", "Rename dance failed: {}", re);
                        return Err(format!(
                            "Cannot replace {}: file may be in use",
                            dest.display()
                        ));
                    }
                }
            }
        }
    }
    dlog!("download", "Moving {} -> {}", tmp.display(), dest.display());
    std::fs::rename(&tmp, dest).or_else(|rename_err| -> Result<(), String> {
        dlog!("download", "Rename failed ({}), falling back to copy", rename_err);
        std::fs::copy(&tmp, dest).map(|bytes| {
            dlog!("download", "Copied {} bytes tmp -> dest", bytes);
        }).map_err(|copy_err| {
            // Rename dance may have left original at .old — restore it so the
            // user isn't left without any binary at `dest`.
            dlog!("download", "Copy failed: {}", copy_err);
            let old = dest.with_extension("old");
            if old.exists() {
                dlog!("download", "Restoring original binary from .old");
                match std::fs::rename(&old, dest) {
                    Ok(_) => dlog!("download", "Restored .old -> dest"),
                    Err(re) => dlog!("download", "Restore failed: {}", re),
                }
            }
            format!("Copy failed: {}", copy_err)
        })?;
        match std::fs::remove_file(&tmp) {
            Ok(_) => dlog!("download", "Removed tmp after copy"),
            Err(e) => dlog!("download", "Failed to remove tmp: {}", e),
        }
        Ok(())
    })?;

    let old = dest.with_extension("old");
    match std::fs::remove_file(&old) {
        Ok(_) => dlog!("download", "Cleaned up .old"),
        Err(e) => dlog!("download", ".old cleanup: {} (ok if nonexistent)", e),
    }

    dlog!("download", "download_to_path complete: {}", dest.display());
    Ok(())
}
