/*
  crates/aiqiezi-server/src/main.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

mod config;
mod endpoint;
mod error;
mod json;
mod logging;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

use crate::config::Config;
use crate::error::AppError;

#[derive(Debug, Parser)]
#[command(
    name = "aiqiezi-server",
    version,
    about = "aiqiezi photo/video archive server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server process
    Run,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    let config = Config::from_env()?;
    logging::init(&config.log_level).map_err(AppError::Logging)?;

    tracing::info!(
        data_dir = %config.data_dir.display(),
        auto_migrate = config.auto_migrate,
        "configuration loaded"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| AppError::Runtime(anyhow::Error::from(error)))?;

    runtime.block_on(async {
        match cli.command.unwrap_or(Command::Run) {
            Command::Run => endpoint::run(&config).await.map_err(AppError::Runtime),
        }
    })
}
