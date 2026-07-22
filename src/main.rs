mod asr;
mod audio;
mod config;
mod daemon;
mod decision;
mod doctor;
mod feedback;
mod history;
mod hotkey;
mod output;
mod paths;
mod session;
mod state;

#[cfg(test)]
mod test_env;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "sage")]
#[command(version, about = "Sage — voice to text for Linux", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start recording (background worker)
    Start,
    /// Stop recording, transcribe, deliver text
    Stop,
    /// Cancel current recording without transcribing
    Cancel,
    /// Show session status
    Status,
    /// Print last transcript
    Last,
    /// Copy last transcript to clipboard
    Copy,
    /// Transcribe an existing WAV
    Transcribe { path: PathBuf },
    /// Model management
    Model {
        #[command(subcommand)]
        action: ModelCmd,
    },
    /// History
    History {
        #[command(subcommand)]
        action: HistoryCmd,
    },
    /// Configuration
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Hotkey shortcut config
    Shortcut {
        #[command(subcommand)]
        action: ShortcutCmd,
    },
    /// Long-running control daemon (stdin commands)
    Daemon,
    /// Show whether daemon is running
    #[command(name = "daemon-status")]
    DaemonStatus,
    /// Environment diagnostics
    Doctor,
    /// Print resolved filesystem paths
    Paths,
    /// Internal recorder worker
    #[command(name = "__worker", hide = true)]
    Worker,
}

#[derive(Subcommand, Debug)]
enum ModelCmd {
    Pull,
    Path,
}

#[derive(Subcommand, Debug)]
enum HistoryCmd {
    /// List recent transcripts
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Clear,
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    Show,
    Get { key: String },
    Set { key: String, value: String },
}

#[derive(Subcommand, Debug)]
enum ShortcutCmd {
    Show,
    Set { chord: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Start => session::cmd_start(),
        Commands::Stop => session::cmd_stop(),
        Commands::Cancel => session::cmd_cancel(),
        Commands::Status => session::cmd_status(),
        Commands::Last => session::cmd_last(),
        Commands::Copy => session::cmd_copy_last(),
        Commands::Transcribe { path } => session::cmd_transcribe(&path),
        Commands::Model { action } => match action {
            ModelCmd::Pull => session::cmd_model_pull(),
            ModelCmd::Path => session::cmd_model_path(),
        },
        Commands::History { action } => match action {
            HistoryCmd::List { limit } => session::cmd_history_list(limit),
            HistoryCmd::Clear => session::cmd_history_clear(),
        },
        Commands::Config { action } => match action {
            ConfigCmd::Show => session::cmd_config_show(),
            ConfigCmd::Get { key } => session::cmd_config_get(&key),
            ConfigCmd::Set { key, value } => session::cmd_config_set(&key, &value),
        },
        Commands::Shortcut { action } => match action {
            ShortcutCmd::Show => session::cmd_shortcut_show(),
            ShortcutCmd::Set { chord } => session::cmd_shortcut_set(&chord),
        },
        Commands::Daemon => daemon::cmd_daemon(),
        Commands::DaemonStatus => daemon::daemon_status(),
        Commands::Doctor => doctor::run(),
        Commands::Paths => cmd_paths(),
        Commands::Worker => session::run_worker(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_paths() -> Result<(), session::SessionError> {
    paths::ensure_dirs().map_err(session::SessionError::from)?;
    println!("config_dir:     {}", paths::config_dir().display());
    println!("config_file:    {}", paths::config_file().display());
    println!("data_dir:       {}", paths::data_dir().display());
    println!("recordings_dir: {}", paths::recordings_dir().display());
    println!("models_dir:     {}", paths::models_dir().display());
    println!("history_file:   {}", paths::history_path().display());
    println!("run_dir:        {}", paths::run_dir().display());
    println!("state_file:     {}", paths::state_path().display());
    println!("temp_dir:       {}", paths::temp_dir().display());
    println!("model_file:     {}", asr::default_model_path().display());
    Ok(())
}
