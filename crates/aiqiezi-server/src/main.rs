/*
  crates/aiqiezi-server/src/main.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

mod admin;
mod admin_client;
mod auth;
mod auth_handler;
mod config;
mod daemon;
mod endpoint;
mod error;
mod firewall;
mod json;
mod lock;
mod logging;
mod shutdown;
mod version;

use clap::{Parser, Subcommand};
use serde_json::json;
use std::process::ExitCode;

use crate::config::Config;
use crate::error::AppError;
use crate::lock::InstanceLock;

#[derive(Debug, Parser)]
#[command(
    name = "aiqiezi-server",
    version = crate::version::VERSION,
    about = "aiqiezi photo/video archive server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the server process in the foreground
    #[command(visible_alias = "run")]
    Start,
    /// Print the server's persistent EndpointId
    Whoami,
    /// Set or change the server password through the running daemon
    SetPassword {
        /// Use the offline path; the daemon must be stopped
        #[arg(long)]
        offline: bool,
        /// Keep existing enrolled clients
        #[arg(long)]
        keep_clients: bool,
    },
    /// Query the running daemon status
    Status,
    /// Check daemon health
    Healthcheck,
    /// Ask the running daemon to shut down
    Shutdown,
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
        match cli.command.unwrap_or(Command::Start) {
            Command::Start => daemon::run(&config).await.map_err(AppError::Runtime),
            Command::Whoami => run_whoami(&config),
            Command::SetPassword {
                offline,
                keep_clients,
            } => run_set_password(&config, offline, keep_clients).await,
            Command::Status => run_admin(&config, "status").await,
            Command::Healthcheck => run_admin(&config, "healthcheck").await,
            Command::Shutdown => run_admin(&config, "shutdown").await,
        }
    })
}

fn run_whoami(config: &Config) -> Result<(), AppError> {
    let key = endpoint::load_key(config).map_err(AppError::Runtime)?;
    println!("{}", key.public());
    Ok(())
}

async fn run_set_password(
    config: &Config,
    offline: bool,
    keep_clients: bool,
) -> Result<(), AppError> {
    let old_password = if auth::password_hash_exists(&config.keys_dir) {
        Some(
            rpassword::prompt_password("current password: ")
                .map_err(|error| AppError::Runtime(anyhow::Error::from(error)))?,
        )
    } else {
        None
    };
    let new_password = rpassword::prompt_password("new password: ")
        .map_err(|error| AppError::Runtime(anyhow::Error::from(error)))?;
    let confirmation = rpassword::prompt_password("confirm new password: ")
        .map_err(|error| AppError::Runtime(anyhow::Error::from(error)))?;

    if new_password != confirmation {
        return Err(AppError::Runtime(anyhow::anyhow!("passwords do not match")));
    }

    if !offline {
        let response = admin_client::request_with_params(
            config,
            "set-password",
            json!({
                "old_password": old_password.clone().unwrap_or_default(),
                "new_password": new_password,
                "keep_clients": keep_clients,
            }),
        )
        .await
        .map_err(|error| {
            AppError::Runtime(error.context(
                "set-password requires the running daemon; use --offline only when it is stopped",
            ))
        })?;

        if let Some(error) = response.error {
            return Err(AppError::Admin {
                code: error.code,
                message: error.message,
            });
        }

        if let Some(message) = response
            .data
            .as_ref()
            .and_then(|data| data.get("message"))
            .and_then(|value| value.as_str())
        {
            println!("{message}");
        } else {
            println!("password changed");
        }
        return Ok(());
    }

    let _lock = InstanceLock::acquire(&config.runtime_dir).map_err(AppError::Runtime)?;

    auth::set_password_offline(
        &config.keys_dir,
        old_password.as_deref(),
        &new_password,
        keep_clients,
    )
    .map_err(AppError::Runtime)?;

    if keep_clients {
        println!("password changed; existing clients remain authorized");
    } else {
        println!("password changed; all clients must authenticate again");
    }
    Ok(())
}

async fn run_admin(config: &Config, command: &str) -> Result<(), AppError> {
    let response = admin_client::request(config, command)
        .await
        .map_err(AppError::Runtime)?;
    println!(
        "{}",
        serde_json::to_string(&response)
            .map_err(|error| AppError::Runtime(anyhow::Error::from(error)))?
    );

    if let Some(error) = response.error {
        return Err(AppError::Admin {
            code: error.code,
            message: error.message,
        });
    }
    Ok(())
}
