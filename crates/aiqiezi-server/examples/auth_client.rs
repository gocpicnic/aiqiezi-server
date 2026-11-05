/*
  crates/aiqiezi-server/examples/auth_client.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

//! Small development client for testing the bootstrap auth protocol.
//!
//! Passwords are read from environment variables so they never appear in argv:
//!
//! ```text
//! AIQIEZI_PASSWORD=... AIQIEZI_NEW_PASSWORD=... \
//!   auth_client --ticket-file ... --data-dir ... flow
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiqiezi_proto::{error_codes, Request, Response, ALPN_AUTH, ALPN_JSON, PROTOCOL_VERSION};
use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Parser)]
#[command(
    name = "auth-client",
    about = "Development client for the aiqiezi bootstrap auth protocol"
)]
struct Cli {
    /// Path to the server's endpoint_ticket.txt
    #[arg(long)]
    ticket_file: PathBuf,

    /// Directory for the persistent test client key
    #[arg(long, default_value = "./data/auth-client")]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the client EndpointId
    Whoami,
    /// Log in with AIQIEZI_PASSWORD
    Login,
    /// Change the password with AIQIEZI_OLD_PASSWORD and AIQIEZI_NEW_PASSWORD
    ChangePassword,
    /// Request server-info over the JSON ALPN
    ServerInfo,
    /// Request status over the JSON ALPN
    Status,
    /// Log in, change the bootstrap password when required, then request server-info
    Flow,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let key = load_or_create_key(&cli.data_dir)?;
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(key)
        .bind()
        .await
        .context("bind client endpoint")?;
    let ticket = read_ticket(&cli.ticket_file)?;

    let result = match cli.command {
        Command::Whoami => {
            println!("{}", endpoint.id());
            Ok(())
        }
        Command::Login => {
            let password = env_required("AIQIEZI_PASSWORD")?;
            let response =
                auth_request(&endpoint, &ticket, "login", json!({"password": password})).await?;
            print_response(&response)?;
            ensure_ok(&response)
        }
        Command::ChangePassword => {
            let old_password = env_required("AIQIEZI_OLD_PASSWORD")?;
            let new_password = env_required("AIQIEZI_NEW_PASSWORD")?;
            let response = auth_request(
                &endpoint,
                &ticket,
                "change-password",
                json!({
                    "old_password": old_password,
                    "new_password": new_password,
                }),
            )
            .await?;
            print_response(&response)?;
            ensure_ok(&response)
        }
        Command::ServerInfo => {
            let response = json_request(&endpoint, &ticket, "server-info", json!({})).await?;
            print_response(&response)?;
            ensure_ok(&response)
        }
        Command::Status => {
            let response = json_request(&endpoint, &ticket, "status", json!({})).await?;
            print_response(&response)?;
            ensure_ok(&response)
        }
        Command::Flow => flow(&endpoint, &ticket).await,
    };

    endpoint.close().await;
    result
}

async fn flow(endpoint: &Endpoint, ticket: &EndpointTicket) -> anyhow::Result<()> {
    let bootstrap_password = env_required("AIQIEZI_PASSWORD")?;
    let new_password = env_required("AIQIEZI_NEW_PASSWORD")?;

    let login = auth_request(
        endpoint,
        ticket,
        "login",
        json!({"password": bootstrap_password}),
    )
    .await?;
    print_response(&login)?;

    match &login.error {
        Some(error) if error.code == error_codes::PASSWORD_CHANGE_REQUIRED => {
            let changed = auth_request(
                endpoint,
                ticket,
                "change-password",
                json!({
                    "old_password": bootstrap_password,
                    "new_password": new_password,
                }),
            )
            .await?;
            print_response(&changed)?;
            ensure_ok(&changed)?;

            let login =
                auth_request(endpoint, ticket, "login", json!({"password": new_password})).await?;
            print_response(&login)?;
            ensure_ok(&login)?;
        }
        Some(_) => bail!("initial login failed"),
        None => {}
    }

    let info = json_request(endpoint, ticket, "server-info", json!({})).await?;
    print_response(&info)?;
    ensure_ok(&info)
}

fn read_ticket(path: &Path) -> anyhow::Result<EndpointTicket> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read endpoint ticket {}", path.display()))?;
    raw.trim()
        .parse::<EndpointTicket>()
        .with_context(|| format!("parse endpoint ticket {}", path.display()))
}

fn load_or_create_key(data_dir: &Path) -> anyhow::Result<SecretKey> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("create client data directory {}", data_dir.display()))?;
    set_directory_permissions(data_dir)?;

    let key_path = data_dir.join("secret_key.bin");
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)
            .with_context(|| format!("read client key {}", key_path.display()))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "client key {} has invalid length: expected 32 bytes, got {}",
                key_path.display(),
                bytes.len()
            )
        })?;
        set_file_permissions(&key_path)?;
        return Ok(SecretKey::from_bytes(&bytes));
    }

    let key = SecretKey::generate();
    std::fs::write(&key_path, key.to_bytes())
        .with_context(|| format!("write client key {}", key_path.display()))?;
    set_file_permissions(&key_path)?;
    Ok(key)
}

fn set_directory_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}

fn set_file_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}

fn env_required(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is not set"))?;
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

async fn auth_request(
    endpoint: &Endpoint,
    ticket: &EndpointTicket,
    command: &str,
    params: Value,
) -> anyhow::Result<Response> {
    request(endpoint, ticket, ALPN_AUTH, command, params).await
}

async fn json_request(
    endpoint: &Endpoint,
    ticket: &EndpointTicket,
    command: &str,
    params: Value,
) -> anyhow::Result<Response> {
    request(endpoint, ticket, ALPN_JSON, command, params).await
}

async fn request(
    endpoint: &Endpoint,
    ticket: &EndpointTicket,
    alpn: &[u8],
    command: &str,
    params: Value,
) -> anyhow::Result<Response> {
    let remote_addr = ticket.endpoint_addr().clone();
    tokio::time::timeout(REQUEST_TIMEOUT, async move {
        let connection = endpoint
            .connect(remote_addr, alpn)
            .await
            .context("connect to server")?;
        let (mut send, recv) = connection.open_bi().await.context("open stream")?;
        let request = Request {
            version: PROTOCOL_VERSION,
            request_id: Some(format!("auth-client-{}", std::process::id())),
            command: command.to_string(),
            params,
        };
        let mut encoded = serde_json::to_vec(&request).context("encode request")?;
        encoded.push(b'\n');
        send.write_all(&encoded).await.context("write request")?;
        send.flush().await.context("flush request")?;

        let mut reader = BufReader::new(recv);
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await.context("read response")?;
        if bytes == 0 {
            bail!("server closed the stream without a response");
        }
        let response = serde_json::from_str::<Response>(&line).context("parse response")?;
        connection.close(0u32.into(), b"auth-client done");
        Ok(response)
    })
    .await
    .context("request timed out")?
}

fn print_response(response: &Response) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(response).context("encode response for output")?
    );
    Ok(())
}

fn ensure_ok(response: &Response) -> anyhow::Result<()> {
    if response.ok {
        return Ok(());
    }

    bail!(
        "server returned {}",
        response
            .error
            .as_ref()
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "an unknown error".to_string())
    )
}
