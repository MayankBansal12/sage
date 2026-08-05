mod asr;
mod audio;
mod config;
mod daemon;
mod doctor;
mod feedback;
mod global_hotkey;
mod hotkey;
mod ipc;
mod models;
mod output;
mod paths;
mod session;
mod store;
mod target;

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
    /// Toggle recording: first invocation starts, second stops
    Toggle,
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
    /// Show queued, transcribing, ready, and failed jobs
    Pending {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Deliver the oldest ready transcript to the focused field or clipboard
    #[command(name = "paste-next")]
    PasteNext,
    /// Cancel a queued or pending job
    #[command(name = "cancel-job")]
    CancelJob { id: String },
    /// Retry a failed or interrupted transcription
    Retry { id: String },
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
    /// Long-running recording, transcription, and delivery daemon
    Daemon,
    /// Show whether daemon is running
    #[command(name = "daemon-status")]
    DaemonStatus,
    /// Stop the background daemon
    #[command(name = "daemon-stop")]
    DaemonStop,
    /// Environment diagnostics
    Doctor,
    /// Print resolved filesystem paths
    Paths,
    /// Internal background daemon entrypoint
    #[command(name = "__daemon", hide = true)]
    BackgroundDaemon,
}

#[derive(Subcommand, Debug)]
enum ModelCmd {
    /// List curated speech-to-text models
    List,
    /// Show details for a catalog model
    Info { id: String },
    /// Download a model without selecting it
    Pull {
        id: String,
        #[arg(long)]
        quant: Option<String>,
    },
    /// Download if needed and select a model
    Use {
        id: String,
        #[arg(long)]
        quant: Option<String>,
        #[arg(long)]
        language: Option<String>,
    },
    /// Show the active model
    Current,
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
        Commands::Toggle => session::cmd_toggle(),
        Commands::Stop => session::cmd_stop(),
        Commands::Cancel => session::cmd_cancel(),
        Commands::Status => session::cmd_status(),
        Commands::Last => session::cmd_last(),
        Commands::Copy => session::cmd_copy_last(),
        Commands::Transcribe { path } => session::cmd_transcribe(&path),
        Commands::Model { action } => match action {
            ModelCmd::List => session::cmd_model_list(),
            ModelCmd::Info { id } => session::cmd_model_info(&id),
            ModelCmd::Pull { id, quant } => session::cmd_model_pull(&id, quant.as_deref()),
            ModelCmd::Use {
                id,
                quant,
                language,
            } => session::cmd_model_use(&id, quant.as_deref(), language.as_deref()),
            ModelCmd::Current => session::cmd_model_current(),
        },
        Commands::History { action } => match action {
            HistoryCmd::List { limit } => session::cmd_history_list(limit),
            HistoryCmd::Clear => session::cmd_history_clear(),
        },
        Commands::Pending { limit } => session::cmd_pending(limit),
        Commands::PasteNext => session::cmd_paste_next(),
        Commands::CancelJob { id } => session::cmd_cancel_job(&id),
        Commands::Retry { id } => session::cmd_retry(&id),
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
        Commands::DaemonStop => session::cmd_daemon_stop(),
        Commands::Doctor => doctor::run(),
        Commands::Paths => cmd_paths(),
        Commands::BackgroundDaemon => daemon::run_background(),
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
    println!("database_file:  {}", paths::database_path().display());
    println!("run_dir:        {}", paths::run_dir().display());
    println!("daemon_socket:  {}", paths::daemon_socket_path().display());
    println!("temp_dir:       {}", paths::temp_dir().display());
    match asr::selected() {
        Ok(selection) => println!("model_file:     {}", selection.path.display()),
        Err(_) => println!("model_file:     (no model selected)"),
    }
    Ok(())
}
