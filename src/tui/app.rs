use crate::core::config::{Config, TokenBotInfo};
use crate::core::platform;
use crate::core::version;
use crate::core::ProgressMsg;
use crate::service::{self, ServiceStatus};
use std::collections::BTreeSet;
use std::sync::mpsc;

/// Result of an asynchronous service-status query.
///
/// Produced by the background status thread; consumed by the TUI main loop.
/// Keeping this off the main thread avoids blocking the UI for the 1–3 s that
/// `Get-ScheduledTask` / `tasklist` take to spawn on Windows.
pub struct StatusUpdate {
    pub service_status: ServiceStatus,
    pub running_token_count: Option<usize>,
}

pub struct TokenInfoUpdate {
    pub token: String,
    pub result: Result<TokenBotInfo, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Welcome,
    TokenInput,
    BinaryPathInput,
    Progress,
    Dashboard,
    LogFullscreen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProgressAction {
    Install,
    Update,
}

#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
    pub expires_at: std::time::Instant,
}

pub struct App {
    pub running: bool,
    pub view: View,
    pub cokacdir_version: Option<String>,
    pub latest_version: Option<String>,
    pub cokacdir_path: Option<String>,
    pub service_status: ServiceStatus,
    pub config: Config,
    pub log_lines: Vec<String>,
    pub log_scroll_offset: usize,
    pub log_viewport_height: std::cell::Cell<u16>,
    pub status_message: Option<StatusMessage>,
    pub checking_update: bool,
    pub token_input: String,
    pub token_list: Vec<String>,
    pub token_disabled: Vec<bool>,
    pub token_cursor: Option<usize>,
    pub running_token_count: Option<usize>,
    pub service_busy: bool,
    pub service_busy_label: String,
    pub service_busy_tick: usize,
    pub service_action_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// Receives `StatusUpdate`s produced by the background status thread.
    pub status_update_rx: Option<mpsc::Receiver<StatusUpdate>>,
    /// Sends "please refresh now" pings to the background status thread.
    /// Send errors (channel closed) are ignored — the thread exits when App drops.
    pub status_refresh_tx: Option<mpsc::Sender<()>>,
    pub token_info_update_tx: mpsc::Sender<TokenInfoUpdate>,
    pub token_info_update_rx: Option<mpsc::Receiver<TokenInfoUpdate>>,
    pub token_info_fetching: BTreeSet<String>,
    // Binary path input state
    pub binary_path_input: String,
    // Progress view state
    pub progress_action: Option<ProgressAction>,
    pub progress_lines: Vec<String>,
    pub progress_rx: Option<std::sync::mpsc::Receiver<ProgressMsg>>,
    pub progress_done: Option<Result<(), String>>,
}

impl App {
    pub fn new() -> Self {
        dlog!("app", "App::new() - loading config...");
        let config = Config::load();
        dlog!("app", "Config loaded: {} tokens", config.tokens.len());

        dlog!("app", "Finding cokacdir...");
        let cokacdir_path = platform::find_cokacdir();
        dlog!("app", "cokacdir_path: {:?}", cokacdir_path);

        let cokacdir_version = cokacdir_path
            .as_ref()
            .and_then(|p| version::installed_version(p));
        dlog!("app", "cokacdir_version: {:?}", cokacdir_version);

        // Start status polling in the background.
        // On Windows, `status()` shells out to PowerShell + tasklist (1–3s).
        // Doing it here would block the first TUI frame; doing it every 25
        // ticks on the main thread blocks input. The background thread polls
        // every 5 s and also services explicit refresh requests.
        dlog!("app", "Spawning background status thread...");
        let (status_update_tx, status_update_rx) = mpsc::channel::<StatusUpdate>();
        let (status_refresh_tx, status_refresh_rx) = mpsc::channel::<()>();
        let (token_info_update_tx, token_info_update_rx) = mpsc::channel::<TokenInfoUpdate>();
        std::thread::spawn(move || {
            dlog!("app::status_thread", "started");
            loop {
                let t0 = std::time::Instant::now();
                let service_status = service::manager().status();
                let running_token_count = if service_status == ServiceStatus::Running {
                    platform::ServicePaths::for_current_os().running_token_count()
                } else {
                    None
                };
                dlog!(
                    "app::status_thread",
                    "polled in {:?}: status={:?} rtc={:?}",
                    t0.elapsed(),
                    service_status,
                    running_token_count
                );
                if status_update_tx
                    .send(StatusUpdate {
                        service_status,
                        running_token_count,
                    })
                    .is_err()
                {
                    dlog!("app::status_thread", "rx dropped, exiting");
                    return;
                }
                // Wait for either an explicit refresh trigger or the 5-second
                // periodic tick. Drain any extra pending triggers so a burst
                // of requests only causes one extra poll.
                match status_refresh_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                    Ok(()) => while status_refresh_rx.try_recv().is_ok() {},
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        dlog!("app::status_thread", "trigger tx dropped, exiting");
                        return;
                    }
                }
            }
        });

        // Initial placeholder until the first StatusUpdate arrives.
        // Renders as "Unknown (Checking...)" in the dashboard.
        let service_status = ServiceStatus::Unknown("Checking...".into());
        let running_token_count = None;

        let initial_view = if cokacdir_path.is_some() {
            View::Dashboard
        } else {
            View::Welcome
        };
        dlog!("app", "Initial view: {:?}", initial_view);

        App {
            running: true,
            view: initial_view,
            cokacdir_version,
            latest_version: None,
            cokacdir_path: cokacdir_path.map(|p| p.to_string_lossy().to_string()),
            service_status,
            running_token_count,
            config,
            log_lines: Vec::new(),
            log_scroll_offset: 0,
            log_viewport_height: std::cell::Cell::new(0),
            status_message: None,
            checking_update: true,
            token_input: String::new(),
            token_list: Vec::new(),
            token_disabled: Vec::new(),
            token_cursor: None,
            progress_action: None,
            progress_lines: Vec::new(),
            progress_rx: None,
            progress_done: None,
            service_busy: false,
            service_busy_label: String::new(),
            service_busy_tick: 0,
            service_action_rx: None,
            status_update_rx: Some(status_update_rx),
            status_refresh_tx: Some(status_refresh_tx),
            token_info_update_tx,
            token_info_update_rx: Some(token_info_update_rx),
            token_info_fetching: BTreeSet::new(),
            binary_path_input: String::new(),
        }
    }

    /// Ask the background status thread for a fresh service status, and
    /// reload the config synchronously (config is a small local JSON file).
    ///
    /// This used to call `service::manager().status()` directly, which on
    /// Windows blocks the UI for 1–3 s spawning PowerShell + tasklist. Now
    /// the heavy work happens off-thread and the result arrives later via
    /// `poll_status_update()`.
    pub fn refresh_status(&mut self) {
        dlog!("app", "refresh_status() — pinging status thread");
        if let Some(tx) = &self.status_refresh_tx {
            let _ = tx.send(());
        }
        self.config = Config::load();
        dlog!(
            "app",
            "Config loaded: total={} active={} disabled={}",
            self.config.tokens.len(),
            self.config.active_tokens().len(),
            self.config.disabled_tokens.len()
        );
    }

    /// Drain pending status updates from the background thread.
    /// Returns true if at least one update was applied.
    pub fn poll_status_update(&mut self) -> bool {
        let mut applied = false;
        let mut disconnected = false;
        // Borrow `status_update_rx` only inside this `if let` so the borrow is
        // released before we (potentially) write to the same field below.
        // Mutating `service_status` / `running_token_count` inside the loop is
        // OK because Rust's disjoint-field borrow rule treats them as separate
        // fields from `status_update_rx`.
        if let Some(rx) = self.status_update_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(update) => {
                        self.service_status = update.service_status;
                        self.running_token_count = update.running_token_count;
                        applied = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            dlog!("app", "status thread disconnected");
            self.status_update_rx = None;
        }
        if applied {
            dlog!(
                "app",
                "status update applied: status={:?} rtc={:?}",
                self.service_status,
                self.running_token_count
            );
        }
        applied
    }

    pub fn refresh_cokacdir_info(&mut self) {
        dlog!("app", "refresh_cokacdir_info()");
        let cokacdir_path = platform::find_cokacdir();
        self.cokacdir_version = cokacdir_path
            .as_ref()
            .and_then(|p| version::installed_version(p));
        self.cokacdir_path = cokacdir_path.map(|p| p.to_string_lossy().to_string());
        dlog!(
            "app",
            "cokacdir version: {:?}, path: {:?}",
            self.cokacdir_version,
            self.cokacdir_path
        );
        self.refresh_status();
    }

    pub fn set_status(&mut self, msg: &str, is_error: bool) {
        dlog!("app", "set_status: '{}' (error: {})", msg, is_error);
        let duration = if is_error { 3 } else { 1 };
        self.status_message = Some(StatusMessage {
            text: msg.to_string(),
            is_error,
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(duration),
        });
    }

    pub fn expire_status(&mut self) {
        if let Some(msg) = &self.status_message {
            if std::time::Instant::now() >= msg.expires_at {
                self.status_message = None;
            }
        }
    }

    pub fn update_available(&self) -> bool {
        if let (Some(latest), Some(current)) = (&self.latest_version, &self.cokacdir_version) {
            version::is_newer(latest, current)
        } else {
            false
        }
    }

    pub fn token_count(&self) -> usize {
        if self.service_status.is_running() {
            self.running_token_count
                .unwrap_or(self.config.active_tokens().len())
        } else {
            self.config.active_tokens().len()
        }
    }

    pub fn max_log_scroll_offset(&self) -> usize {
        let visible = self.log_viewport_height.get() as usize;
        if visible == 0 {
            self.log_lines.len().saturating_sub(1)
        } else {
            self.log_lines.len().saturating_sub(visible)
        }
    }

    pub fn enter_binary_path_input(&mut self) {
        dlog!("app", "enter_binary_path_input()");
        self.binary_path_input = self.config.install_path.clone().unwrap_or_default();
        self.view = View::BinaryPathInput;
    }

    pub fn enter_token_input(&mut self) {
        dlog!("app", "enter_token_input()");
        self.config = Config::load();
        self.token_input.clear();
        self.token_list = self.config.tokens.clone();
        self.token_disabled = self
            .config
            .tokens
            .iter()
            .map(|t| self.config.disabled_tokens.contains(t))
            .collect();
        self.token_cursor = None;
        self.view = View::TokenInput;
        self.fetch_missing_token_info();
    }

    pub fn fetch_missing_token_info(&mut self) {
        let missing: Vec<String> = self
            .token_list
            .iter()
            .filter(|token| {
                !self.config.token_bot_info.contains_key(*token)
                    && !self.token_info_fetching.contains(*token)
            })
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }

        for token in &missing {
            self.token_info_fetching.insert(token.clone());
        }
        self.set_status("Fetching Telegram bot info...", false);
        dlog!(
            "app::token_info",
            "fetching {} missing token metadata item(s)",
            missing.len()
        );

        let tx = self.token_info_update_tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    for token in missing {
                        let _ = tx.send(TokenInfoUpdate {
                            token,
                            result: Err(format!("Cannot start Telegram lookup runtime: {}", e)),
                        });
                    }
                    return;
                }
            };

            for token in missing {
                let result = match rt.block_on(crate::core::telegram::fetch_bot_info(&token)) {
                    Ok(info) => {
                        let save_result = save_token_info_if_still_registered(&token, &info);
                        match save_result {
                            Ok(()) => Ok(info),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                };
                let _ = tx.send(TokenInfoUpdate { token, result });
            }
        });
    }

    pub fn poll_token_info_update(&mut self) -> bool {
        let mut updates = Vec::new();
        let mut disconnected = false;
        if let Some(rx) = self.token_info_update_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            dlog!("app::token_info", "token info update channel disconnected");
            self.token_info_update_rx = None;
        }
        if updates.is_empty() {
            return false;
        }

        let mut success_count = 0usize;
        let mut error_count = 0usize;
        for update in updates {
            self.token_info_fetching.remove(&update.token);
            match update.result {
                Ok(info) => {
                    if self.token_list.iter().any(|t| t == &update.token)
                        || self.config.tokens.iter().any(|t| t == &update.token)
                    {
                        self.config.token_bot_info.insert(update.token, info);
                    }
                    success_count += 1;
                }
                Err(e) => {
                    dlog!("app::token_info", "Telegram info fetch failed: {}", e);
                    error_count += 1;
                }
            }
        }

        if success_count > 0 {
            self.set_status("Telegram bot info updated", false);
        } else if error_count > 0 {
            self.set_status("Could not fetch Telegram bot info", true);
        }
        true
    }

    pub fn start_progress(&mut self, action: ProgressAction) {
        dlog!("app", "start_progress({:?})", action);
        let (tx, rx) = std::sync::mpsc::channel();
        self.progress_action = Some(action.clone());
        self.progress_lines.clear();
        self.progress_done = None;
        self.progress_rx = Some(rx);
        self.view = View::Progress;

        match action {
            ProgressAction::Install => {
                dlog!("app", "Spawning install thread");
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let _ = rt.block_on(crate::cli::install::run_bg(tx));
                });
            }
            ProgressAction::Update => {
                dlog!("app", "Spawning update thread");
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let _ = rt.block_on(crate::cli::update::run_bg(tx));
                });
            }
        }
    }

    /// Poll progress channel, returns true if there were new messages.
    pub fn poll_progress(&mut self) -> bool {
        let rx = match &self.progress_rx {
            Some(rx) => rx,
            None => return false,
        };
        let mut got_any = false;
        loop {
            match rx.try_recv() {
                Ok(ProgressMsg::Log(line)) => {
                    dlog!("app", "Progress log: {}", line);
                    self.progress_lines.push(line);
                    got_any = true;
                }
                Ok(ProgressMsg::Done(result)) => {
                    dlog!("app", "Progress done: {:?}", result);
                    self.progress_done = Some(result);
                    got_any = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    dlog!("app", "Progress channel disconnected");
                    if self.progress_done.is_none() {
                        self.progress_done = Some(Err("Operation terminated unexpectedly.".into()));
                    }
                    break;
                }
            }
        }
        got_any
    }

    /// Poll service action result from background thread.
    pub fn poll_service_action(&mut self) {
        let rx = match &self.service_action_rx {
            Some(rx) => rx,
            None => return,
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                dlog!("app", "Service action succeeded");
                self.service_action_rx = None;
                self.service_busy = false;
                self.set_status("Service operation completed", false);
                self.refresh_status();
            }
            Ok(Err(e)) => {
                dlog!("app", "Service action failed: {}", e);
                self.service_action_rx = None;
                self.service_busy = false;
                for line in e.lines() {
                    self.log_lines.push(line.to_string());
                }
                self.set_status(&format!("Failed: {}", e), true);
                self.refresh_status();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.service_busy_tick += 1;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                dlog!("app", "Service action channel disconnected");
                self.service_action_rx = None;
                self.service_busy = false;
            }
        }
    }
}

fn save_token_info_if_still_registered(token: &str, info: &TokenBotInfo) -> Result<(), String> {
    let mut config = Config::load();
    if !config.tokens.iter().any(|t| t == token) {
        dlog!(
            "app::token_info",
            "skip saving metadata for token no longer registered"
        );
        return Ok(());
    }
    config
        .token_bot_info
        .insert(token.to_string(), info.clone());
    config.save()
}
