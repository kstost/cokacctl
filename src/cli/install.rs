use crate::core::{download, platform, ProgressMsg, ProgressTx};

const COKACDIR_INSTALL_SH_URL: &str = "https://cokacdir.cokac.com/install.sh";
const PATH_MARKER: &str = "# cokacdir PATH (added by installer)";
const PATH_BLOCK: &str = "\n# cokacdir PATH (added by installer)\ncase \":$PATH:\" in\n    *\":$HOME/.local/bin:\"*) ;;\n    *) export PATH=\"$HOME/.local/bin:$PATH\" ;;\nesac\n";
const SHELL_WRAPPER_BEGIN: &str = "# BEGIN COKACDIR SHELL WRAPPER";
const SHELL_WRAPPER_END: &str = "# END COKACDIR SHELL WRAPPER";
const CURRENT_WRAPPER_MARKER: &str =
    r#"COKACDIR_LASTDIR_FILE="$cokacdir_lastdir_file" command cokacdir "$@""#;
const SHELL_WRAPPER_HEADER: &str = "# cokacdir - cd to last directory on interactive exit";
const LEGACY_LASTDIR_WRAPPER_MARKER: &str = r#"cat ~/.cokacdir/lastdir"#;
const LEGACY_INTERACTIVE_WRAPPER_MARKER: &str = "local cokacdir_should_cd=1";

fn send(tx: &Option<ProgressTx>, msg: String) {
    if let Some(tx) = tx {
        tx.send(ProgressMsg::Log(msg)).ok();
    } else {
        println!("{}", msg);
    }
}

/// Best-effort service restart when an install step failed after the service
/// was stopped — keeps the user from ending up with a silently-down service.
/// Uses whatever binary is currently findable (old binary on plain failure,
/// or restored-from-.old binary on partial replacement).
fn try_restart_existing(tx: &Option<ProgressTx>) {
    dlog!("install", "try_restart_existing: begin");
    let config = crate::core::config::Config::load();
    let tokens = config.active_tokens();
    if tokens.is_empty() {
        dlog!("install", "try_restart_existing: no active tokens — skip");
        return;
    }
    if let Some(existing) = platform::find_cokacdir() {
        dlog!(
            "install",
            "Rollback: restarting with {}",
            existing.display()
        );
        send(
            tx,
            "  Install failed — restarting service with existing binary...".into(),
        );
        match crate::service::manager().start(&existing, &tokens) {
            Ok(_) => dlog!("install", "Rollback: service restart ok"),
            Err(e) => dlog!("install", "Rollback: service restart failed: {}", e),
        }
    } else {
        dlog!(
            "install",
            "Rollback: no existing binary found, cannot restart"
        );
    }
}

/// CLI entry point (prints to stdout).
pub async fn run() -> Result<(), String> {
    dlog!("install", "CLI run()");
    run_inner(&None).await
}

/// TUI entry point (sends progress via channel).
pub async fn run_bg(tx: ProgressTx) -> Result<(), String> {
    dlog!("install", "TUI run_bg()");
    let tx_opt = Some(tx);
    let result = run_inner(&tx_opt).await;
    if let Some(tx) = &tx_opt {
        tx.send(ProgressMsg::Done(result.clone())).ok();
    }
    dlog!("install", "run_bg() result: {:?}", result);
    result
}

async fn run_inner(tx: &Option<ProgressTx>) -> Result<(), String> {
    let os = platform::Os::detect();
    let arch = platform::Arch::detect();
    let url = platform::binary_download_url(os, arch);
    let install_path = platform::default_install_path(os);
    let shell_wrapper = if os != platform::Os::Windows {
        Some(fetch_canonical_shell_wrapper(tx).await?)
    } else {
        None
    };

    dlog!("install", "OS: {:?}, Arch: {:?}", os, arch);
    dlog!("install", "URL: {}", url);
    dlog!("install", "Install path: {}", install_path.display());

    send(
        tx,
        format!(
            "  Installing cokacdir ({}-{})...",
            os.as_str(),
            arch.as_str()
        ),
    );
    send(tx, format!("  Source: {}", url));
    send(tx, format!("  Target: {}", install_path.display()));

    // Stop service if running (binary may be locked, especially on Windows)
    dlog!("install", "Checking service status...");
    let mgr = crate::service::manager();
    let status = mgr.status();
    let was_running = status.is_running() || mgr.is_any_running();
    dlog!("install", "Service was_running: {}", was_running);
    if was_running {
        send(tx, "  Stopping running service...".into());
        dlog!("install", "Stopping service...");
        mgr.stop().ok();
    }

    // Try default path, fallback if not writable
    let dest = if os != platform::Os::Windows {
        if let Some(parent) = install_path.parent() {
            if !is_writable(parent) {
                dlog!("install", "Default path not writable, trying sudo");
                send(tx, "  /usr/local/bin requires elevated privileges.".into());
                send(tx, "  Trying sudo...".into());
                return install_with_sudo(
                    &url,
                    &install_path,
                    was_running,
                    tx,
                    shell_wrapper
                        .as_deref()
                        .expect("Unix install should have a shell wrapper"),
                )
                .await;
            }
        }
        install_path.clone()
    } else {
        install_path.clone()
    };

    dlog!("install", "Downloading to: {}", dest.display());
    if let Err(e) = download::download_to_path(&url, &dest, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }

    // Setup shell wrapper on Unix
    if os != platform::Os::Windows {
        dlog!("install", "Setting up shell wrapper...");
        setup_shell_wrapper_inner(
            tx,
            &dest,
            shell_wrapper
                .as_deref()
                .expect("Unix install should have a shell wrapper"),
        );
    }

    send(tx, format!("  cokacdir installed at {}", dest.display()));
    dlog!("install", "Install complete at {}", dest.display());

    // Restart service if it was running
    if was_running {
        let config = crate::core::config::Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("install", "Restarting service...");
            send(tx, "  Restarting service...".into());
            mgr.start(&dest, &tokens).ok();
        }
    }

    Ok(())
}

async fn install_with_sudo(
    url: &str,
    dest: &std::path::Path,
    was_running: bool,
    tx: &Option<ProgressTx>,
    shell_wrapper: &str,
) -> Result<(), String> {
    dlog!("install", "install_with_sudo()");
    let tmp = std::env::temp_dir().join(format!("cokacdir_dl_{}", std::process::id()));
    if let Err(e) = download::download_to_path(url, &tmp, tx).await {
        if was_running {
            try_restart_existing(tx);
        }
        return Err(e);
    }

    let mut cmd = std::process::Command::new("sudo");
    if tx.is_some() {
        cmd.arg("-n");
    }
    let sudo_label = format!(
        "sudo{} mv {} {}",
        if tx.is_some() { " -n" } else { "" },
        tmp.display(),
        dest.display()
    );
    dlog!("install", "Invoking: {}", sudo_label);
    let status = match cmd
        .args(["mv", &tmp.to_string_lossy(), &dest.to_string_lossy()])
        .status()
    {
        Ok(s) => {
            crate::core::debug::log_status("install", &sudo_label, &s);
            s
        }
        Err(e) => {
            dlog!("install", "sudo mv exec failed: {}", e);
            std::fs::remove_file(&tmp).ok();
            if was_running {
                try_restart_existing(tx);
            }
            return Err(format!("sudo mv failed: {}", e));
        }
    };

    let actual_path = if !status.success() {
        let fallback = platform::fallback_install_path();
        dlog!(
            "install",
            "sudo failed, falling back to {}",
            fallback.display()
        );
        send(
            tx,
            format!(
                "  sudo failed. Installing to {} instead.",
                fallback.display()
            ),
        );
        if let Some(parent) = fallback.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Err(e) = std::fs::rename(&tmp, &fallback).or_else(|_| -> Result<(), String> {
            std::fs::copy(&tmp, &fallback)
                .map(|_| ())
                .map_err(|e| format!("Copy failed: {}", e))
        }) {
            std::fs::remove_file(&tmp).ok();
            if was_running {
                try_restart_existing(tx);
            }
            return Err(e);
        }
        std::fs::remove_file(&tmp).ok();
        send(
            tx,
            format!("  cokacdir installed at {}", fallback.display()),
        );
        fallback
    } else {
        dlog!("install", "sudo mv succeeded");
        send(tx, format!("  cokacdir installed at {}", dest.display()));
        dest.to_path_buf()
    };

    setup_shell_wrapper_inner(tx, &actual_path, shell_wrapper);

    if was_running {
        let config = crate::core::config::Config::load();
        let tokens = config.active_tokens();
        if !tokens.is_empty() {
            dlog!("install", "Restarting service after sudo install...");
            send(tx, "  Restarting service...".into());
            crate::service::manager().start(&actual_path, &tokens).ok();
        }
    }

    Ok(())
}

fn is_writable(path: &std::path::Path) -> bool {
    if !path.exists() {
        dlog!(
            "install",
            "is_writable: {} does not exist -> false",
            path.display()
        );
        return false;
    }
    let test_file = path.join(".cokacctl_write_test");
    match std::fs::write(&test_file, b"") {
        Ok(_) => {
            std::fs::remove_file(&test_file).ok();
            dlog!("install", "is_writable: {} -> true", path.display());
            true
        }
        Err(e) => {
            dlog!(
                "install",
                "is_writable: {} -> false ({})",
                path.display(),
                e
            );
            false
        }
    }
}

fn canonical_install_sh_url() -> String {
    std::env::var("COKACCTL_INSTALL_SH_URL").unwrap_or_else(|_| COKACDIR_INSTALL_SH_URL.to_string())
}

async fn fetch_canonical_shell_wrapper(tx: &Option<ProgressTx>) -> Result<String, String> {
    let url = canonical_install_sh_url();
    send(tx, format!("  Fetching shell wrapper from {}", url));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch shell wrapper from {}: {}", url, e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch shell wrapper from {}: HTTP {}",
            url,
            response.status()
        ));
    }

    let script = response
        .text()
        .await
        .map_err(|e| format!("Failed to read shell wrapper from {}: {}", url, e))?;

    extract_canonical_shell_wrapper(&script)
        .ok_or_else(|| format!("No valid cokacdir shell wrapper block found in {}", url))
}

fn extract_canonical_shell_wrapper(script: &str) -> Option<String> {
    let begin = script.find(SHELL_WRAPPER_BEGIN)?;
    let after_begin = &script[begin..];
    let end = after_begin.find(SHELL_WRAPPER_END)? + SHELL_WRAPPER_END.len();
    let block = &after_begin[..end];

    if !block.contains("cokacdir()") || !block.contains(CURRENT_WRAPPER_MARKER) {
        return None;
    }

    let mut wrapper = block.trim_matches(|c| c == '\n' || c == '\r').to_string();
    wrapper.push('\n');
    Some(wrapper)
}

struct ShellWrapperUpdate {
    content: String,
    changed: bool,
    wrapper_written: bool,
    custom_function_present: bool,
}

fn update_shell_wrapper_content(existing: &str, shell_wrapper: &str) -> ShellWrapperUpdate {
    let (mut content, custom_function_present) = strip_managed_cokacdir_wrappers(existing);

    let wrapper_written = !custom_function_present;
    if wrapper_written {
        append_current_shell_wrapper(&mut content, shell_wrapper);
    }

    ShellWrapperUpdate {
        changed: content != existing,
        content,
        wrapper_written,
        custom_function_present,
    }
}

fn strip_managed_cokacdir_wrappers(existing: &str) -> (String, bool) {
    let lines: Vec<&str> = existing.split_inclusive('\n').collect();
    let mut output = String::new();
    let mut custom_function_present = false;
    let mut i = 0;

    while i < lines.len() {
        if lines[i].trim() == SHELL_WRAPPER_BEGIN {
            let (block, end) = collect_marked_wrapper_block(&lines, i);
            if is_managed_cokacdir_wrapper(&block) {
                i = end;
                continue;
            }
            if block.contains("cokacdir()") {
                custom_function_present = true;
            }
            output.push_str(&block);
            i = end;
            continue;
        }

        if is_shell_wrapper_header(lines[i]) {
            if let Some(next) = next_non_empty_line(&lines, i + 1) {
                if is_cokacdir_function_start(lines[next]) {
                    let (block, end) = collect_function_block(&lines, next);
                    if is_managed_cokacdir_wrapper(&block) {
                        i = end;
                        continue;
                    }
                }
            }
        }

        if is_cokacdir_function_start(lines[i]) {
            let (block, end) = collect_function_block(&lines, i);
            if is_managed_cokacdir_wrapper(&block) {
                i = end;
                continue;
            }
            custom_function_present = true;
            output.push_str(&block);
            i = end;
            continue;
        }

        output.push_str(lines[i]);
        i += 1;
    }

    (output, custom_function_present)
}

fn append_current_shell_wrapper(content: &mut String, shell_wrapper: &str) {
    while content.ends_with("\n\n") {
        content.pop();
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(shell_wrapper.trim_end());
    content.push('\n');
}

fn next_non_empty_line(lines: &[&str], start: usize) -> Option<usize> {
    (start..lines.len()).find(|&idx| !lines[idx].trim().is_empty())
}

fn is_shell_wrapper_header(line: &str) -> bool {
    line.trim() == SHELL_WRAPPER_HEADER
        || line.trim() == "# cokacdir - cd to last directory on exit"
}

fn is_cokacdir_function_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("cokacdir()") && trimmed.contains('{')
}

fn collect_function_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut block = String::new();
    let mut depth = 0isize;
    let mut idx = start;

    while idx < lines.len() {
        let line = lines[idx];
        depth += line.matches('{').count() as isize;
        depth -= line.matches('}').count() as isize;
        block.push_str(line);
        idx += 1;
        if depth <= 0 {
            break;
        }
    }

    (block, idx)
}

fn collect_marked_wrapper_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut block = String::new();
    let mut idx = start;

    while idx < lines.len() {
        let line = lines[idx];
        block.push_str(line);
        idx += 1;
        if line.trim() == SHELL_WRAPPER_END {
            break;
        }
    }

    (block, idx)
}

fn is_managed_cokacdir_wrapper(block: &str) -> bool {
    block.contains(CURRENT_WRAPPER_MARKER)
        || block.contains(LEGACY_LASTDIR_WRAPPER_MARKER)
        || block.contains(LEGACY_INTERACTIVE_WRAPPER_MARKER)
}

fn setup_shell_wrapper_inner(
    tx: &Option<ProgressTx>,
    install_path: &std::path::Path,
    shell_wrapper: &str,
) {
    let config_path = match platform::shell_config_path() {
        Some(p) => {
            dlog!("install", "setup_shell_wrapper: config={}", p.display());
            p
        }
        None => {
            dlog!(
                "install",
                "setup_shell_wrapper: no shell config path detected, skip"
            );
            send(
                tx,
                "  Could not determine a zsh/bash config file; shell wrapper was not added.".into(),
            );
            return;
        }
    };

    let existing = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    let wrapper_update = update_shell_wrapper_content(&existing, shell_wrapper);

    // Only add the PATH block when we installed under the fallback directory
    // (~/.local/bin). For /usr/local/bin installs the directory is already in
    // PATH on every supported distro, so we don't pollute the user's rc file.
    let fallback_dir: Option<std::path::PathBuf> = platform::fallback_install_path()
        .parent()
        .map(|p| p.to_path_buf());
    let installed_in_fallback_dir = match (fallback_dir.as_ref(), install_path.parent()) {
        (Some(fb), Some(ip)) => fb == ip,
        _ => false,
    };
    let needs_path = installed_in_fallback_dir && !existing.contains(PATH_MARKER);

    if !wrapper_update.changed && !needs_path {
        dlog!(
            "install",
            "Shell config already up to date: {}",
            config_path.display()
        );
        return;
    }

    let mut content = wrapper_update.content;
    if needs_path {
        content.push_str(PATH_BLOCK);
    }

    match std::fs::write(&config_path, &content) {
        Ok(_) => {
            if wrapper_update.wrapper_written && wrapper_update.changed {
                dlog!(
                    "install",
                    "Shell wrapper added to {}",
                    config_path.display()
                );
                send(
                    tx,
                    format!("  Shell wrapper added to {}", config_path.display()),
                );
                if !needs_path {
                    send(
                        tx,
                        format!(
                            "  Open a new terminal (or run: source {}) to enable the cokacdir shell function.",
                            config_path.display()
                        ),
                    );
                }
            }
            if wrapper_update.custom_function_present {
                send(
                    tx,
                    "  Existing custom cokacdir shell function found; left it unchanged.".into(),
                );
            }
            if needs_path {
                dlog!("install", "PATH export added to {}", config_path.display());
                send(
                    tx,
                    format!("  Added ~/.local/bin to PATH in {}", config_path.display()),
                );
                send(
                    tx,
                    format!(
                        "  Open a new terminal (or run: source {}) to apply.",
                        config_path.display()
                    ),
                );
            }
        }
        Err(e) => {
            dlog!(
                "install",
                "shell config write failed for {}: {}",
                config_path.display(),
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_wrapper_fixture() -> &'static str {
        r#"# BEGIN COKACDIR SHELL WRAPPER
# cokacdir - cd to last directory on interactive exit
cokacdir() {
    local cokacdir_lastdir_file
    COKACDIR_LASTDIR_FILE="$cokacdir_lastdir_file" command cokacdir "$@"
}
# END COKACDIR SHELL WRAPPER
"#
    }

    fn count_cokacdir_functions(content: &str) -> usize {
        content.matches("cokacdir()").count()
    }

    #[test]
    fn shell_wrapper_extracted_from_install_script() {
        let script = format!(
            r#"write_shell_wrapper() {{
    cat <<'COKACDIR_SHELL_WRAPPER'
{}COKACDIR_SHELL_WRAPPER
}}"#,
            shell_wrapper_fixture()
        );

        assert_eq!(
            extract_canonical_shell_wrapper(&script).as_deref(),
            Some(shell_wrapper_fixture())
        );
    }

    #[test]
    fn shell_wrapper_extract_rejects_missing_markers() {
        assert!(
            extract_canonical_shell_wrapper("cokacdir() { command cokacdir \"$@\"; }").is_none()
        );
    }

    #[test]
    fn shell_wrapper_extract_rejects_missing_current_marker() {
        let script = r#"# BEGIN COKACDIR SHELL WRAPPER
cokacdir() {
    command cokacdir "$@"
}
# END COKACDIR SHELL WRAPPER
"#;

        assert!(extract_canonical_shell_wrapper(script).is_none());
    }

    #[test]
    fn shell_wrapper_added_when_missing() {
        let update = update_shell_wrapper_content("", shell_wrapper_fixture());

        assert!(update.changed);
        assert!(update.wrapper_written);
        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
    }

    #[test]
    fn shell_wrapper_current_wrapper_stays_single() {
        let existing = shell_wrapper_fixture().to_string();
        let update = update_shell_wrapper_content(&existing, shell_wrapper_fixture());

        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
    }

    #[test]
    fn shell_wrapper_env_var_comment_does_not_count_as_current_wrapper() {
        let update = update_shell_wrapper_content(
            "# COKACDIR_LASTDIR_FILE=example\n",
            shell_wrapper_fixture(),
        );

        assert!(update.changed);
        assert!(update.wrapper_written);
        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
    }

    #[test]
    fn shell_wrapper_replaces_legacy_lastdir_wrapper() {
        let legacy = r#"cokacdir() { command cokacdir "$@" && cd "$(cat ~/.cokacdir/lastdir 2>/dev/null || pwd)"; }"#;
        let update = update_shell_wrapper_content(legacy, shell_wrapper_fixture());

        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
        assert!(!update.content.contains(LEGACY_LASTDIR_WRAPPER_MARKER));
    }

    #[test]
    fn shell_wrapper_replaces_legacy_interactive_wrapper() {
        let legacy = r#"cokacdir() {
    local cokacdir_should_cd=1
    command cokacdir "$@"
}"#;
        let update = update_shell_wrapper_content(legacy, shell_wrapper_fixture());

        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
        assert!(!update.content.contains(LEGACY_INTERACTIVE_WRAPPER_MARKER));
    }

    #[test]
    fn shell_wrapper_leaves_unknown_custom_wrapper_unchanged() {
        let custom = r#"cokacdir() {
    echo "custom"
    command cokacdir "$@"
}"#;
        let update = update_shell_wrapper_content(custom, shell_wrapper_fixture());

        assert!(!update.changed);
        assert!(!update.wrapper_written);
        assert!(update.custom_function_present);
        assert_eq!(update.content, custom);
    }

    #[test]
    fn shell_wrapper_deduplicates_current_and_legacy_wrappers() {
        let legacy = r#"cokacdir() { command cokacdir "$@" && cd "$(cat ~/.cokacdir/lastdir 2>/dev/null || pwd)"; }
"#;
        let existing = format!("{}\n{}", shell_wrapper_fixture(), legacy);
        let update = update_shell_wrapper_content(&existing, shell_wrapper_fixture());

        assert_eq!(count_cokacdir_functions(&update.content), 1);
        assert!(update.content.contains(CURRENT_WRAPPER_MARKER));
        assert!(!update.content.contains(LEGACY_LASTDIR_WRAPPER_MARKER));
    }
}
