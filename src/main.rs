//! Sage — Linux voice-to-text.
//!
//! Phase 0: skeleton · Phase 1: recording · Phase 2: transcription

mod asr;
mod audio;
mod config;
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
    /// Start recording from the default microphone (background worker)
    Start,
    /// Stop recording, save WAV, and transcribe
    Stop,
    /// Show session status (idle/recording), last paths, transcript
    Status,
    /// Print the last transcript
    Last,
    /// Transcribe an existing audio file (WAV)
    Transcribe {
        /// Path to a WAV file
        path: PathBuf,
    },
    /// Model management
    Model {
        #[command(subcommand)]
        action: ModelCmd,
    },
    /// Print resolved filesystem paths
    Paths,
    /// Internal: background recorder worker (do not invoke directly)
    #[command(name = "__worker", hide = true)]
    Worker,
}

#[derive(Subcommand, Debug)]
enum ModelCmd {
    /// Download the default whisper tiny model
    Pull,
    /// Show the model path and whether it exists
    Path,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Start => session::cmd_start(),
        Commands::Stop => session::cmd_stop(),
        Commands::Status => session::cmd_status(),
        Commands::Last => session::cmd_last(),
        Commands::Transcribe { path } => session::cmd_transcribe(&path),
        Commands::Model { action } => match action {
            ModelCmd::Pull => session::cmd_model_pull(),
            ModelCmd::Path => session::cmd_model_path(),
        },
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
    println!("run_dir:        {}", paths::run_dir().display());
    println!("state_file:     {}", paths::state_path().display());
    println!("temp_dir:       {}", paths::temp_dir().display());
    println!("model_file:     {}", asr::default_model_path().display());
    Ok(())
}
