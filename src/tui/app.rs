use crate::core::config::Config;
use crate::core::platform;
use crate::core::version;
use crate::core::ProgressMsg;
use crate::service::{self, ServiceStatus};

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

        dlog!("app", "Querying initial service status...");
        let service_status = service::manager().status();
        dlog!("app", "Service status: {:?}", service_status);
        let running_token_count = if service_status == ServiceStatus::Running {
            platform::ServicePaths::for_current_os().running_token_count()
        } else {
            None
        };

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
            binary_path_input: String::new(),
        }
    }

    pub fn refresh_status(&mut self) {
        dlog!("app", "refresh_status()");
        self.service_status = service::manager().status();
        dlog!("app", "Service status: {:?}", self.service_status);
        self.config = Config::load();
        dlog!("app", "Config loaded: total={} active={} disabled={}",
            self.config.tokens.len(),
            self.config.active_tokens().len(),
            self.config.disabled_tokens.len());
        self.running_token_count = if self.service_status == ServiceStatus::Running {
            let rtc = platform::ServicePaths::for_current_os().running_token_count();
            dlog!("app", "running_token_count result: {:?}", rtc);
            rtc
        } else {
            dlog!("app", "running_token_count: None (not Running)");
            None
        };
        dlog!("app", "final token_count() = {}", self.token_count());
    }

    pub fn refresh_cokacdir_info(&mut self) {
        dlog!("app", "refresh_cokacdir_info()");
        let cokacdir_path = platform::find_cokacdir();
        self.cokacdir_version = cokacdir_path
            .as_ref()
            .and_then(|p| version::installed_version(p));
        self.cokacdir_path = cokacdir_path.map(|p| p.to_string_lossy().to_string());
        dlog!("app", "cokacdir version: {:?}, path: {:?}", self.cokacdir_version, self.cokacdir_path);
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
        if self.service_status == ServiceStatus::Running {
            self.running_token_count.unwrap_or(self.config.active_tokens().len())
        } else {
            self.config.active_tokens().len()
        }
    }

    pub fn enter_binary_path_input(&mut self) {
        dlog!("app", "enter_binary_path_input()");
        self.binary_path_input = self.config.install_path.clone().unwrap_or_default();
        self.view = View::BinaryPathInput;
    }

    pub fn enter_token_input(&mut self) {
        dlog!("app", "enter_token_input()");
        self.token_input.clear();
        self.token_list = self.config.tokens.clone();
        self.token_disabled = self.config.tokens.iter()
            .map(|t| self.config.disabled_tokens.contains(t))
            .collect();
        self.token_cursor = None;
        self.view = View::TokenInput;
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
