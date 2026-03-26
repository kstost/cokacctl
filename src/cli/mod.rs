pub mod install;
pub mod update;
pub mod service;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cokacctl")]
#[command(about = "cokacdir installation and service manager")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install cokacdir binary
    Install,
    /// Update cokacdir to the latest version
    Update,
    /// Manage the cokacdir background service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Show cokacdir and cokacctl version info
    Status,
}

#[derive(Debug, Subcommand)]
pub enum ServiceAction {
    /// Register and start the service
    Start {
        /// Telegram bot tokens
        #[arg(required = true)]
        tokens: Vec<String>,
    },
    /// Stop the service
    Stop,
    /// Restart the service
    Restart,
    /// Remove the service
    Remove,
    /// Show service status
    Status,
    /// Tail the service log
    Log,
    /// Change bot tokens (restarts service)
    Token {
        /// New Telegram bot tokens
        #[arg(required = true)]
        tokens: Vec<String>,
    },
}
