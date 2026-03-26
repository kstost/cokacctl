pub mod platform;
pub mod version;
pub mod download;
pub mod config;

/// Progress message for background operations displayed in TUI.
pub enum ProgressMsg {
    Log(String),
    Done(Result<(), String>),
}

pub type ProgressTx = std::sync::mpsc::Sender<ProgressMsg>;
