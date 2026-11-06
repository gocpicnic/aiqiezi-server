/*
  crates/aiqiezi-server/src/admin_client.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use aiqiezi_proto::{Request, Response, PROTOCOL_VERSION};
use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::config::Config;

pub async fn request(config: &Config, command: &str) -> anyhow::Result<Response> {
    request_with_params(config, command, json!({})).await
}

pub async fn request_with_params(
    config: &Config,
    command: &str,
    params: Value,
) -> anyhow::Result<Response> {
    let socket_path = config.admin_socket_path();
    let stream = UnixStream::connect(&socket_path).await.with_context(|| {
        format!(
            "connect to admin socket {}; is aiqiezi-server running?",
            socket_path.display()
        )
    })?;
    let (read_half, mut write_half) = stream.into_split();

    let request = Request {
        version: PROTOCOL_VERSION,
        request_id: Some(format!("cli-{}", std::process::id())),
        command: command.to_string(),
        params,
    };
    let mut encoded = serde_json::to_vec(&request)
        .map_err(|error| anyhow::anyhow!("encode admin request: {error}"))?;
    encoded.push(b'\n');
    write_half
        .write_all(&encoded)
        .await
        .context("write admin request")?;
    write_half.flush().await.context("flush admin request")?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .await
        .context("read admin response")?;
    if bytes == 0 {
        anyhow::bail!("admin socket closed without a response");
    }

    serde_json::from_str(&line).context("parse admin response")
}
