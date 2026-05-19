//! JSON API handlers for the dashboard.
//!
//! Bots are addressed by an opaque `id` derived from the token. The raw token
//! never leaves the server — neither in `/api/state` responses nor in any
//! mutation endpoint payload. This keeps tokens out of network traffic, the
//! browser memory of network observers, and any access logs that might land
//! between the dashboard and an inbound client.

use std::hash::{Hash, Hasher};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::config::Config;
use crate::core::platform::{self, Os};
use crate::core::version;
use crate::core::{ProgressMsg, ProgressTx};
use crate::service::{self, ServiceStatus};

use super::server::{Request, Response};
use super::state::{rfc3339_systime, SharedState};

const MAX_TOKEN_LEN: usize = 512;
const MAX_NAME_LEN: usize = 128;
const MAX_PATH_LEN: usize = 1024;

pub async fn handle(req: &Request, state: &SharedState) -> Response {
    dlog!("api", "dispatch {} {} body={}B peer={}",
          req.method, req.path, req.body.len(), req.peer);
    let resp = match (req.method.as_str(), req.path.as_str()) {
        ("GET",  "/api/state")            => get_state(state).await,
        ("GET",  "/api/logs")             => get_logs().await,
        ("GET",  "/api/activity")         => get_activity(state).await,
        ("POST", "/api/service/start")    => post_service(state, ServiceAction::Start).await,
        ("POST", "/api/service/stop")     => post_service(state, ServiceAction::Stop).await,
        ("POST", "/api/service/restart")  => post_service(state, ServiceAction::Restart).await,
        ("POST", "/api/service/remove")   => post_service(state, ServiceAction::Remove).await,
        ("POST", "/api/install")          => post_install(state).await,
        ("POST", "/api/update/check")     => post_check_update(state).await,
        ("POST", "/api/update/apply")     => post_apply_update(state).await,
        ("POST", "/api/tokens/add")       => post_token_add(state, &req.body).await,
        ("POST", "/api/tokens/toggle")    => post_token_toggle(state, &req.body).await,
        ("POST", "/api/tokens/delete")    => post_token_delete(state, &req.body).await,
        ("POST", "/api/binary-path")      => post_binary_path(state, &req.body).await,
        _ => {
            dlog!("api", "no handler for {} {}", req.method, req.path);
            Response::not_found()
        }
    };
    dlog!("api", "response {} {} -> {} ({}B)",
          req.method, req.path, resp.status, resp.body.len());
    resp
}

// ─── GET /api/state ────────────────────────────────────────────────────────

async fn get_state(state: &SharedState) -> Response {
    dlog!("api::state", "building state snapshot");
    let body = tokio::task::spawn_blocking({
        let state = state.clone();
        move || build_state_json(&state)
    })
    .await
    .unwrap_or_else(|e| {
        dlog!("api::state", "spawn_blocking join failed: {}", e);
        json!({ "error": format!("join: {}", e) })
    });
    let s = body.to_string();
    dlog!("api::state", "snapshot ready ({}B)", s.len());
    Response::ok_json(s)
}

fn build_state_json(state: &SharedState) -> Value {
    let os = Os::detect();
    let config = Config::load();
    dlog!("api::state", "os={:?} tokens={} disabled={}",
          os, config.tokens.len(), config.disabled_tokens.len());

    let mgr = service::manager();
    let svc_status_raw = mgr.status();
    let svc_status = match svc_status_raw {
        ServiceStatus::Running       => "running",
        ServiceStatus::Stopped       => "stopped",
        ServiceStatus::NotInstalled  => "not-installed",
        ServiceStatus::Unknown(_)    => "unknown",
    };
    dlog!("api::state", "service status: {}", svc_status);

    let binary_path = platform::find_cokacdir()
        .map(|p| p.display().to_string());
    dlog!("api::state", "binary path: {:?}", binary_path);

    let cokacdir_version = binary_path.as_ref().and_then(|p| {
        version::installed_version(std::path::Path::new(p))
    });
    dlog!("api::state", "cokacdir version: {:?}", cokacdir_version);

    let svc_paths = platform::ServicePaths::for_current_os();
    let log_path = svc_paths.log_file.display().to_string();
    let error_log_path = svc_paths.error_log_file.display().to_string();
    let config_path = Config::path().display().to_string();
    let debug_log_path = dirs::home_dir()
        .map(|h| h.join(".cokacdir").join("debug").join("cokacctl.log").display().to_string())
        .unwrap_or_default();

    let platform_obj = json!({
        "id":    platform_id(os),
        "label": service_label(os),
        "host":  hostname(),
        "os":    os_label(os),
    });

    let bots: Vec<Value> = config.tokens.iter().enumerate().map(|(i, token)| {
        let disabled = config.disabled_tokens.contains(token);
        let name = config.token_names
            .get(token)
            .cloned()
            .unwrap_or_else(|| derive_bot_name(token, i));
        let handle = derive_bot_handle(token);
        json!({
            "id":       bot_id(token),
            "name":     name,
            "handle":   handle,
            "preview":  mask_token(token),
            "disabled": disabled,
            "addedAt":  Value::Null,
        })
    }).collect();

    json!({
        "serviceStatus":    svc_status,
        "cokacctlVersion":  env!("CARGO_PKG_VERSION"),
        "cokacdirVersion":  cokacdir_version,
        "latestVersion":    state.latest_version(),
        "lastCheck":        state.last_check().map(rfc3339_systime),
        "checkingUpdate":   state.checking(),
        "platform":         platform_obj,
        "binaryPath":       binary_path,
        "configPath":       config_path,
        "logPath":          log_path,
        "errorLogPath":     error_log_path,
        "debugLogPath":     debug_log_path,
        "bots":             bots,
        "startedAt":        state.started_at().map(rfc3339_systime),
        "autoCheckUpdate":  false,
        "inbound":          state.inbound(),
    })
}

fn platform_id(os: Os) -> &'static str {
    match os {
        Os::MacOS   => "macos",
        Os::Linux   => "linux",
        Os::Windows => "windows",
    }
}

fn service_label(os: Os) -> &'static str {
    match os {
        Os::MacOS   => "launchd",
        Os::Linux   => "systemd",
        Os::Windows => "Task Scheduler",
    }
}

fn os_label(os: Os) -> &'static str {
    match os {
        Os::MacOS   => "macOS",
        Os::Linux   => "Linux",
        Os::Windows => "Windows",
    }
}

fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = vec![0u8; 256];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) == 0 {
                if let Some(end) = buf.iter().position(|&b| b == 0) {
                    buf.truncate(end);
                }
                if let Ok(s) = String::from_utf8(buf) {
                    if !s.is_empty() { return s; }
                }
            }
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

fn derive_bot_name(token: &str, idx: usize) -> String {
    if let Some((id, _)) = token.split_once(':') {
        format!("Bot {}", id)
    } else {
        format!("Bot {}", idx + 1)
    }
}

fn derive_bot_handle(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    let tail: String = chars.iter().rev().take(6).collect::<Vec<_>>()
        .into_iter().rev().collect();
    format!("@bot_{}", tail)
}

// ─── GET /api/logs ─────────────────────────────────────────────────────────

async fn get_logs() -> Response {
    dlog!("api::logs", "reading service log");
    let body = tokio::task::spawn_blocking(|| {
        let mgr = service::manager();
        let path = match mgr.log_path() {
            Some(p) => p,
            None => {
                dlog!("api::logs", "log_path returned None");
                return json!({ "lines": [] });
            }
        };
        if !path.exists() {
            dlog!("api::logs", "log file does not exist: {}", path.display());
            return json!({ "lines": [] });
        }
        dlog!("api::logs", "loading from {}", path.display());
        let lines = crate::tui::log_viewer::load_log_lines(&path, 200);
        dlog!("api::logs", "loaded {} lines", lines.len());
        let parsed: Vec<Value> = lines.iter().enumerate().map(|(i, raw)| {
            json!({
                "id":     format!("l-{}-{}", i, raw.len()),
                "time":   super::state::rfc3339_now(),
                "level":  classify_level(raw),
                "source": "cokacdir",
                "msg":    raw,
            })
        }).collect();
        json!({ "lines": parsed })
    })
    .await
    .unwrap_or_else(|e| {
        dlog!("api::logs", "spawn_blocking join failed: {}", e);
        json!({ "error": format!("join: {}", e) })
    });

    Response::ok_json(body.to_string())
}

fn classify_level(line: &str) -> &'static str {
    let l = line.to_lowercase();
    if l.contains(" error") || l.contains("[error]") || l.contains("fatal") { "err" }
    else if l.contains(" warn") || l.contains("[warn]")                      { "warn" }
    else if l.contains(" debug") || l.contains("[debug]")                    { "debug" }
    else if l.contains(" ok ") || l.contains("[ok]")                          { "ok" }
    else                                                                      { "info" }
}

// ─── GET /api/activity ─────────────────────────────────────────────────────

async fn get_activity(state: &SharedState) -> Response {
    let items = state.activity();
    dlog!("api::activity", "returning {} items", items.len());
    Response::ok_json(json!({ "items": items }).to_string())
}

// ─── POST /api/service/* ───────────────────────────────────────────────────

enum ServiceAction { Start, Stop, Restart, Remove }

async fn post_service(state: &SharedState, action: ServiceAction) -> Response {
    let action_name = match action {
        ServiceAction::Start   => "start",
        ServiceAction::Stop    => "stop",
        ServiceAction::Restart => "restart",
        ServiceAction::Remove  => "remove",
    };
    dlog!("api::service", "action={}", action_name);
    let state = state.clone();
    let result: Result<&'static str, String> = tokio::task::spawn_blocking(move || {
        let mgr = service::manager();
        match action {
            ServiceAction::Start => {
                let config = Config::load();
                let tokens = config.active_tokens();
                dlog!("api::service", "start: active_tokens={}", tokens.len());
                if tokens.is_empty() {
                    dlog!("api::service", "start: refused — no active tokens");
                    return Err("No active tokens. Add one from the Tokens page first.".into());
                }
                let bin = platform::find_cokacdir()
                    .ok_or_else(|| {
                        dlog!("api::service", "start: refused — cokacdir not installed");
                        "cokacdir is not installed. Install it first.".to_string()
                    })?;
                dlog!("api::service", "start: bin={}", bin.display());
                mgr.start(&bin, &tokens).map_err(|e| {
                    dlog!("api::service", "start: mgr.start failed: {}", e);
                    e
                })?;
                state.mark_started();
                state.push_activity("svc-start", "Service started",
                    &format!("Running with {} bot token(s)", tokens.len()), "green");
                dlog!("api::service", "start: done");
                Ok("Service started")
            }
            ServiceAction::Stop => {
                mgr.stop().map_err(|e| {
                    dlog!("api::service", "stop: mgr.stop failed: {}", e);
                    e
                })?;
                state.mark_stopped();
                state.push_activity("svc-stop", "Service stopped", "Manual stop", "red");
                dlog!("api::service", "stop: done");
                Ok("Service stopped")
            }
            ServiceAction::Restart => {
                let config = Config::load();
                let tokens = config.active_tokens();
                dlog!("api::service", "restart: active_tokens={}", tokens.len());
                if tokens.is_empty() {
                    dlog!("api::service", "restart: refused — no active tokens");
                    return Err("No active tokens.".into());
                }
                let bin = platform::find_cokacdir()
                    .ok_or_else(|| {
                        dlog!("api::service", "restart: refused — cokacdir not installed");
                        "cokacdir is not installed.".to_string()
                    })?;
                dlog!("api::service", "restart: bin={}", bin.display());
                mgr.restart(&bin, &tokens).map_err(|e| {
                    dlog!("api::service", "restart: mgr.restart failed: {}", e);
                    e
                })?;
                state.mark_started();
                state.push_activity("svc-restart", "Service restarted",
                    &format!("{} bot token(s)", tokens.len()), "green");
                dlog!("api::service", "restart: done");
                Ok("Restarted")
            }
            ServiceAction::Remove => {
                mgr.remove().map_err(|e| {
                    dlog!("api::service", "remove: mgr.remove failed: {}", e);
                    e
                })?;
                state.mark_stopped();
                state.push_activity("svc-stop", "Service unregistered", "Removed from service manager", "red");
                dlog!("api::service", "remove: done");
                Ok("Service registration removed")
            }
        }
    })
    .await
    .map_err(|e| {
        dlog!("api::service", "spawn_blocking join failed: {}", e);
        format!("join: {}", e)
    })
    .and_then(|r| r);

    match result {
        Ok(msg) => {
            dlog!("api::service", "action={} -> OK ({})", action_name, msg);
            Response::ok_json(json!({ "message": msg }).to_string())
        }
        Err(e) => {
            dlog!("api::service", "action={} -> ERR: {}", action_name, e);
            Response::err_json(400, "Bad Request", e)
        }
    }
}

// ─── POST /api/install ─────────────────────────────────────────────────────

async fn post_install(state: &SharedState) -> Response {
    dlog!("api::install", "request received, trying lock");
    // Only one install/update may touch the cokacdir binary at a time.
    // try_lock so a double-click returns 409 immediately instead of queueing
    // the user behind a multi-minute download.
    let _guard = match state.binary_op_lock.try_lock() {
        Ok(g) => {
            dlog!("api::install", "lock acquired");
            g
        }
        Err(_) => {
            dlog!("api::install", "lock contended -> 409");
            return Response::err_json(
                409, "Conflict",
                "An install or update is already in progress. Please wait for it to finish.".into(),
            );
        }
    };
    // Use the run_bg path so install treats us as non-interactive (sudo -n).
    // Without this, an interactive sudo prompt would deadlock the request.
    let (tx, _rx) = drain_progress();
    dlog!("api::install", "invoking install::run_bg");
    match crate::cli::install::run_bg(tx).await {
        Ok(_) => {
            dlog!("api::install", "install::run_bg succeeded");
            state.push_activity("install", "cokacdir installed", "Latest version", "blue");
            Response::ok_json(json!({ "message": "cokacdir installed" }).to_string())
        }
        Err(e) => {
            dlog!("api::install", "install::run_bg failed: {}", e);
            Response::err_json(500, "Install Failed", e)
        }
    }
}

// ─── POST /api/update/check ───────────────────────────────────────────────

async fn post_check_update(state: &SharedState) -> Response {
    dlog!("api::update_check", "begin");
    state.set_checking(true);
    let latest = version::latest_version().await;
    dlog!("api::update_check", "latest_version -> {:?}", latest);
    state.set_latest_version(latest.clone());
    state.set_checking(false);
    Response::ok_json(json!({ "latestVersion": latest }).to_string())
}

// ─── POST /api/update/apply ───────────────────────────────────────────────

async fn post_apply_update(state: &SharedState) -> Response {
    dlog!("api::update_apply", "request received, trying lock");
    // Shares the mutex with /api/install — two concurrent runs would race on
    // the cokacdir binary.
    let _guard = match state.binary_op_lock.try_lock() {
        Ok(g) => {
            dlog!("api::update_apply", "lock acquired");
            g
        }
        Err(_) => {
            dlog!("api::update_apply", "lock contended -> 409");
            return Response::err_json(
                409, "Conflict",
                "An install or update is already in progress. Please wait for it to finish.".into(),
            );
        }
    };
    let old = tokio::task::spawn_blocking(|| {
        platform::find_cokacdir()
            .and_then(|p| version::installed_version(&p))
    })
    .await
    .ok()
    .flatten();
    dlog!("api::update_apply", "old version: {:?}", old);

    let (tx, _rx) = drain_progress();
    dlog!("api::update_apply", "invoking update::run_bg");
    match crate::cli::update::run_bg(tx).await {
        Ok(_) => {
            dlog!("api::update_apply", "update::run_bg succeeded");
            let new = tokio::task::spawn_blocking(|| {
                platform::find_cokacdir()
                    .and_then(|p| version::installed_version(&p))
            })
            .await
            .ok()
            .flatten();
            dlog!("api::update_apply", "new version: {:?}", new);
            let meta = match (old, new) {
                (Some(a), Some(b)) if a != b => format!("v{} → v{}", a, b),
                (_, Some(b)) => format!("v{}", b),
                _ => "Update complete".to_string(),
            };
            state.push_activity("update", "cokacdir updated", &meta, "blue");
            Response::ok_json(json!({ "message": "Updated" }).to_string())
        }
        Err(e) => {
            dlog!("api::update_apply", "update::run_bg failed: {}", e);
            Response::err_json(500, "Update Failed", e)
        }
    }
}

// ─── POST /api/tokens/* ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct AddBot { token: String, #[serde(default)] name: Option<String> }

#[derive(Deserialize)]
struct BotIdRef { id: String }

async fn post_token_add(state: &SharedState, body: &[u8]) -> Response {
    dlog!("api::token_add", "body={}B", body.len());
    let req: AddBot = match parse_json(body) {
        Ok(r) => r,
        Err(e) => {
            dlog!("api::token_add", "parse failed: {}", e);
            return Response::err_json(400, "Bad Request", e);
        }
    };
    let token = req.token.trim().to_string();
    if let Err(e) = validate_token(&token) {
        dlog!("api::token_add", "validate_token failed: {}", e);
        return Response::err_json(400, "Bad Request", e);
    }
    let name_opt = match req.name {
        Some(ref n) => match validate_name(n) {
            Ok(s) => s,
            Err(e) => {
                dlog!("api::token_add", "validate_name failed: {}", e);
                return Response::err_json(400, "Bad Request", e);
            }
        },
        None => None,
    };
    dlog!("api::token_add", "token={} name={:?}", mask_token(&token), name_opt);
    let state_c = state.clone();
    let result: Result<(), String> = tokio::task::spawn_blocking(move || {
        let _cg = state_c.config_lock.lock().unwrap();
        let mut config = Config::load();
        if config.tokens.iter().any(|t| t == &token) {
            dlog!("api::token_add", "duplicate token");
            return Err("Token already registered".into());
        }
        config.tokens.push(token.clone());
        if let Some(n) = name_opt {
            config.token_names.insert(token.clone(), n);
        }
        config.save().map_err(|e| {
            dlog!("api::token_add", "config.save failed: {}", e);
            e
        })?;
        state_c.push_activity("bot-add", "Bot added", &mask_token(&token), "blue");
        dlog!("api::token_add", "saved (total tokens now {})", config.tokens.len());
        Ok(())
    }).await.unwrap_or_else(|e| {
        dlog!("api::token_add", "spawn_blocking join failed: {}", e);
        Err(format!("join: {}", e))
    });
    match result {
        Ok(_)  => Response::ok_json(json!({ "message": "Bot added" }).to_string()),
        Err(e) => Response::err_json(400, "Bad Request", e),
    }
}

async fn post_token_toggle(state: &SharedState, body: &[u8]) -> Response {
    dlog!("api::token_toggle", "body={}B", body.len());
    let req: BotIdRef = match parse_json(body) {
        Ok(r) => r,
        Err(e) => {
            dlog!("api::token_toggle", "parse failed: {}", e);
            return Response::err_json(400, "Bad Request", e);
        }
    };
    let id = req.id;
    dlog!("api::token_toggle", "id={}", id);
    let state_c = state.clone();
    let result: Result<bool, String> = tokio::task::spawn_blocking(move || {
        let _cg = state_c.config_lock.lock().unwrap();
        let mut config = Config::load();
        let token = match resolve_id(&config.tokens, &id) {
            Some(t) => t,
            None => {
                dlog!("api::token_toggle", "unknown id");
                return Err("Unknown bot".into());
            }
        };
        let was_disabled = config.disabled_tokens.iter().any(|t| t == &token);
        if was_disabled {
            config.disabled_tokens.retain(|t| t != &token);
        } else {
            config.disabled_tokens.push(token.clone());
        }
        config.save().map_err(|e| {
            dlog!("api::token_toggle", "config.save failed: {}", e);
            e
        })?;
        let now_disabled = !was_disabled;
        dlog!("api::token_toggle", "token={} was_disabled={} now_disabled={}",
              mask_token(&token), was_disabled, now_disabled);
        state_c.push_activity(
            if now_disabled { "bot-disable" } else { "bot-add" },
            if now_disabled { "Bot disabled" } else { "Bot enabled" },
            &mask_token(&token),
            if now_disabled { "" } else { "blue" },
        );
        Ok(now_disabled)
    }).await.unwrap_or_else(|e| {
        dlog!("api::token_toggle", "spawn_blocking join failed: {}", e);
        Err(format!("join: {}", e))
    });
    match result {
        Ok(disabled) => Response::ok_json(json!({ "disabled": disabled }).to_string()),
        Err(e)       => Response::err_json(400, "Bad Request", e),
    }
}

async fn post_token_delete(state: &SharedState, body: &[u8]) -> Response {
    dlog!("api::token_delete", "body={}B", body.len());
    let req: BotIdRef = match parse_json(body) {
        Ok(r) => r,
        Err(e) => {
            dlog!("api::token_delete", "parse failed: {}", e);
            return Response::err_json(400, "Bad Request", e);
        }
    };
    let id = req.id;
    dlog!("api::token_delete", "id={}", id);
    let state_c = state.clone();
    let result: Result<(), String> = tokio::task::spawn_blocking(move || {
        let _cg = state_c.config_lock.lock().unwrap();
        let mut config = Config::load();
        let token = match resolve_id(&config.tokens, &id) {
            Some(t) => t,
            None => {
                dlog!("api::token_delete", "unknown id");
                return Err("Unknown bot".into());
            }
        };
        config.tokens.retain(|t| t != &token);
        config.disabled_tokens.retain(|t| t != &token);
        config.token_names.remove(&token);
        config.save().map_err(|e| {
            dlog!("api::token_delete", "config.save failed: {}", e);
            e
        })?;
        dlog!("api::token_delete", "removed token={} (remaining {})",
              mask_token(&token), config.tokens.len());
        state_c.push_activity("bot-remove", "Bot removed", &mask_token(&token), "red");
        Ok(())
    }).await.unwrap_or_else(|e| {
        dlog!("api::token_delete", "spawn_blocking join failed: {}", e);
        Err(format!("join: {}", e))
    });
    match result {
        Ok(_)  => Response::ok_json(json!({ "message": "Bot removed" }).to_string()),
        Err(e) => Response::err_json(400, "Bad Request", e),
    }
}

// ─── POST /api/binary-path ────────────────────────────────────────────────

#[derive(Deserialize)]
struct BinaryPath { path: String }

async fn post_binary_path(state: &SharedState, body: &[u8]) -> Response {
    dlog!("api::binary_path", "body={}B", body.len());
    let req: BinaryPath = match parse_json(body) {
        Ok(r) => r,
        Err(e) => {
            dlog!("api::binary_path", "parse failed: {}", e);
            return Response::err_json(400, "Bad Request", e);
        }
    };
    let trimmed = req.path.trim().to_string();
    dlog!("api::binary_path", "candidate path: {:?}", trimmed);
    // Cheap syntactic checks run on the async task. The filesystem existence
    // check lives inside the spawn_blocking below so a stat on a wedged
    // mount can't stall the tokio reactor.
    if let Err(e) = validate_path_syntax(&trimmed) {
        dlog!("api::binary_path", "syntax check failed: {}", e);
        return Response::err_json(400, "Bad Request", e);
    }
    let state_c = state.clone();
    let result: Result<(), String> = tokio::task::spawn_blocking(move || {
        if let Err(e) = validate_path_exists(&trimmed) {
            dlog!("api::binary_path", "exists check failed: {}", e);
            return Err(e);
        }
        let _cg = state_c.config_lock.lock().unwrap();
        let mut config = Config::load();
        let meta = if trimmed.is_empty() {
            config.install_path = None;
            dlog!("api::binary_path", "auto-detect enabled");
            "Auto-detect enabled".to_string()
        } else {
            let m = trimmed.clone();
            config.install_path = Some(trimmed);
            dlog!("api::binary_path", "install_path set to {}", m);
            m
        };
        config.save().map_err(|e| {
            dlog!("api::binary_path", "config.save failed: {}", e);
            e
        })?;
        state_c.push_activity("install", "Binary path changed", &meta, "blue");
        Ok(())
    }).await.unwrap_or_else(|e| {
        dlog!("api::binary_path", "spawn_blocking join failed: {}", e);
        Err(format!("join: {}", e))
    });
    match result {
        Ok(_)  => Response::ok_json(json!({ "message": "Saved" }).to_string()),
        Err(e) => Response::err_json(400, "Bad Request", e),
    }
}

// ─── helpers ───────────────────────────────────────────────────────────────

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|e| format!("Invalid JSON: {}", e))
}

/// Build a ProgressTx whose receiver is held but unread. The install/update
/// code paths branch on `tx.is_some()` to choose non-interactive sudo (`-n`),
/// so just having the channel — even with an unread rx — is what avoids the
/// hanging password prompt for dashboard-initiated runs. The unbounded mpsc
/// never blocks the sender.
fn drain_progress() -> (ProgressTx, std::sync::mpsc::Receiver<ProgressMsg>) {
    std::sync::mpsc::channel()
}

/// Char-safe redaction: keeps the leading 8 / trailing 6 *characters*, not
/// bytes. Indexing by byte would panic on multi-byte UTF-8 input.
fn mask_token(t: &str) -> String {
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= 16 {
        return t.to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 6..].iter().collect();
    format!("{}……{}", head, tail)
}

/// Stable opaque id derived from the token. The token never leaves the
/// server; clients refer to bots by this id when toggling or deleting.
fn bot_id(token: &str) -> String {
    // SipHash via DefaultHasher gives ~64-bit collision resistance, more than
    // enough for the small set of tokens a single user manages.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn resolve_id(tokens: &[String], id: &str) -> Option<String> {
    tokens.iter().find(|t| bot_id(t) == id).cloned()
}

fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("Token is empty".into());
    }
    if token.len() > MAX_TOKEN_LEN {
        return Err(format!("Token is too long (max {} chars)", MAX_TOKEN_LEN));
    }
    if token.chars().any(|c| c.is_control()) {
        return Err("Token contains control characters".into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<Option<String>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(format!("Name is too long (max {} chars)", MAX_NAME_LEN));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("Name contains control characters".into());
    }
    Ok(Some(trimmed.to_string()))
}

/// Cheap structural checks on the user-supplied install path — length, no
/// control chars, absolute. Empty string is the "auto-detect" sentinel.
fn validate_path_syntax(path: &str) -> Result<(), String> {
    if path.len() > MAX_PATH_LEN {
        return Err(format!("Path is too long (max {} chars)", MAX_PATH_LEN));
    }
    if path.chars().any(|c| c == '\0' || (c.is_control() && c != '\t')) {
        return Err("Path contains control characters".into());
    }
    if path.is_empty() {
        return Ok(());
    }
    if !std::path::Path::new(path).is_absolute() {
        return Err("Path must be absolute (e.g., /usr/local/bin/cokacdir)".into());
    }
    Ok(())
}

/// Filesystem existence check — separate from syntactic validation so the
/// caller can run it on the blocking pool. Closes off the "authenticated
/// user points install_path at an attacker-staged binary, then starts the
/// service" vector: the attacker can no longer reserve an arbitrary string
/// now and arrange the payload later — the file has to be there at save
/// time. `find_cokacdir` re-checks `is_file` at service-start, so a binary
/// that disappears between save and start also fails.
fn validate_path_exists(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    if !std::path::Path::new(path).is_file() {
        return Err("No file at that path. Install cokacdir first or provide a correct path.".into());
    }
    Ok(())
}
