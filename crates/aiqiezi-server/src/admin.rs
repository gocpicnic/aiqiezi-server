/*
  crates/aiqiezi-server/src/admin.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::path::{Path, PathBuf};
use std::time::Instant;

use aiqiezi_proto::{error_codes, Request, Response, PROTOCOL_VERSION};
use anyhow::Context;
use iroh::EndpointId;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::auth::AuthState;
use crate::lock::create_runtime_dir;
use crate::shutdown::Shutdown;
use crate::version;

#[derive(Clone)]
pub struct AdminState {
    pub endpoint_id: EndpointId,
    pub started: Instant,
    pub shutdown: Shutdown,
    pub auth: AuthState,
}

impl AdminState {
    pub fn new(endpoint_id: EndpointId, shutdown: Shutdown, auth: AuthState) -> Self {
        Self {
            endpoint_id,
            started: Instant::now(),
            shutdown,
            auth,
        }
    }

    fn status(&self) -> Value {
        json!({
            "name": "aiqiezi-server",
            "version": version::VERSION,
            "build_date": version::BUILD_DATE,
            "endpoint_id": self.endpoint_id.to_string(),
            "uptime_secs": self.started.elapsed().as_secs(),
            "database": {"status": "not_configured"},
            "migrations": {"status": "not_configured"},
            "media_storage": {"status": "not_configured"},
            "auth": {
                "mode": "password",
                "password_change_required": self.auth.must_change_password(),
                "enabled_clients": self.auth.client_count(),
            },
            "enabled_clients": self.auth.client_count(),
        })
    }

    fn healthcheck(&self) -> Value {
        json!({
            "name": "aiqiezi-server",
            "ok": true,
            "checks": {
                "daemon": "ok",
                "admin_socket": "ok",
                "iroh_endpoint": "ok",
                "auth": "ok",
                "database": "not_configured",
                "migrations": "not_configured",
                "media_storage": "not_configured",
            },
        })
    }
}

pub struct AdminServer {
    listener: UnixListener,
    path: PathBuf,
    state: AdminState,
}

impl AdminServer {
    pub fn bind(path: PathBuf, state: AdminState) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            create_runtime_dir(parent)?;
        }
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("remove stale admin socket {}", path.display()))?;
        }

        let listener = UnixListener::bind(&path)
            .with_context(|| format!("bind admin socket {}", path.display()))?;
        set_socket_permissions(&path)?;

        Ok(Self {
            listener,
            path,
            state,
        })
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                _ = self.state.shutdown.wait() => {
                    info!("admin socket stopping");
                    return Ok(());
                }
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            let state = self.state.clone();
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, state).await {
                                    warn!("admin connection error: {error}");
                                }
                            });
                        }
                        Err(error) => {
                            return Err(error).context("accept admin connection");
                        }
                    }
                }
            }
        }
    }
}

impl Drop for AdminServer {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "failed to remove admin socket {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

fn set_socket_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))
            .with_context(|| format!("set permissions on admin socket {}", path.display()))?;
    }
    Ok(())
}

async fn handle_connection(stream: UnixStream, state: AdminState) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .await
        .context("read admin request")?;
    if bytes == 0 {
        return Ok(());
    }

    let (response, shutdown_after_response) = handle_line(&state, &line);
    let mut encoded = serde_json::to_vec(&response)
        .map_err(|error| anyhow::anyhow!("encode admin response: {error}"))?;
    encoded.push(b'\n');
    write_half
        .write_all(&encoded)
        .await
        .context("write admin response")?;
    write_half.flush().await.context("flush admin response")?;

    if shutdown_after_response {
        state.shutdown.trigger();
    }

    Ok(())
}

fn handle_line(state: &AdminState, line: &str) -> (Response, bool) {
    let request = match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => request,
        Err(error) => {
            return (
                Response::error(
                    None,
                    error_codes::INVALID_REQUEST,
                    format!("invalid JSON request: {error}"),
                ),
                false,
            );
        }
    };

    if request.version != PROTOCOL_VERSION {
        return (
            Response::error(
                request.request_id,
                error_codes::UNSUPPORTED_PROTOCOL,
                format!(
                    "unsupported protocol version {}, expected {}",
                    request.version, PROTOCOL_VERSION
                ),
            ),
            false,
        );
    }

    match request.command.as_str() {
        "status" => (Response::success(request.request_id, state.status()), false),
        "healthcheck" => (
            Response::success(request.request_id, state.healthcheck()),
            false,
        ),
        "shutdown" => (
            Response::success(request.request_id, json!({"message": "shutting down"})),
            true,
        ),
        "reload-whitelist" => (
            Response::error(
                request.request_id,
                error_codes::DATABASE_UNAVAILABLE,
                "client whitelist database is not configured yet",
            ),
            false,
        ),
        _ => (
            Response::error(
                request.request_id,
                error_codes::INVALID_REQUEST,
                format!("unknown admin command: {}", request.command),
            ),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_line, AdminServer, AdminState};
    use crate::auth::{AuthState, BOOTSTRAP_TEST_PASSWORD};
    use crate::shutdown::Shutdown;
    use aiqiezi_proto::{error_codes, Response, PROTOCOL_VERSION};
    use std::time::Instant;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn state() -> AdminState {
        let key = iroh::SecretKey::generate();
        let auth_dir = tempfile::tempdir().unwrap();
        let auth =
            AuthState::load_or_initialize(auth_dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        AdminState {
            endpoint_id: key.public(),
            started: Instant::now(),
            shutdown: Shutdown::new(),
            auth,
        }
    }

    fn request(command: &str) -> String {
        format!(r#"{{"version":{PROTOCOL_VERSION},"command":"{command}"}}"#)
    }

    #[test]
    fn status_returns_safe_fields() {
        let state = state();
        let (response, shutdown) = handle_line(&state, &request("status"));
        assert!(response.ok);
        assert!(!shutdown);
        let data = response.data.unwrap();
        assert_eq!(data["endpoint_id"], state.endpoint_id.to_string());
        assert_eq!(data["database"]["status"], "not_configured");
        assert_eq!(data["auth"]["mode"], "password");
        assert_eq!(data["auth"]["password_change_required"], true);
        assert_eq!(data["auth"]["enabled_clients"], 0);
        assert!(data.get("database_url").is_none());
        assert!(data.get("relay_token").is_none());
        assert!(data.get("password_hash").is_none());
    }

    #[test]
    fn healthcheck_returns_ok() {
        let (response, _) = handle_line(&state(), &request("healthcheck"));
        assert!(response.ok);
        assert_eq!(response.data.unwrap()["ok"], true);
    }

    #[test]
    fn malformed_json_is_invalid_request() {
        let (response, _) = handle_line(&state(), "not-json");
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[test]
    fn unknown_command_is_invalid_request() {
        let (response, _) = handle_line(&state(), &request("unknown"));
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[test]
    fn reload_whitelist_requires_database() {
        let (response, _) = handle_line(&state(), &request("reload-whitelist"));
        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::DATABASE_UNAVAILABLE
        );
    }

    #[test]
    fn shutdown_requests_shutdown_after_response() {
        let (_response, shutdown) = handle_line(&state(), &request("shutdown"));
        assert!(shutdown);
    }

    #[tokio::test]
    async fn stale_socket_is_replaced() {
        use crate::lock::create_runtime_dir;
        use std::os::unix::fs::FileTypeExt;

        let root = tempfile::tempdir().unwrap();
        create_runtime_dir(root.path()).unwrap();
        let socket_path = root.path().join("admin.sock");
        std::fs::write(&socket_path, b"stale").unwrap();

        let server = AdminServer::bind(socket_path.clone(), state()).unwrap();
        let file_type = std::fs::metadata(&socket_path).unwrap().file_type();
        assert!(file_type.is_socket());

        drop(server);
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn socket_serves_a_request_and_has_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let socket_path = root.path().join("admin.sock");
        let state = state();
        let endpoint_id = state.endpoint_id;
        let shutdown = state.shutdown.clone();
        let server = AdminServer::bind(socket_path.clone(), state).unwrap();

        let mode = std::fs::metadata(&socket_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o660);

        let task = tokio::spawn(server.serve());
        let stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        write_half
            .write_all(request("status").as_bytes())
            .await
            .unwrap();
        write_half.write_all(b"\n").await.unwrap();
        write_half.flush().await.unwrap();

        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: Response = serde_json::from_str(&line).unwrap();
        assert!(response.ok);
        assert_eq!(
            response.data.unwrap()["endpoint_id"],
            endpoint_id.to_string()
        );

        shutdown.trigger();
        task.await.unwrap().unwrap();
        assert!(!socket_path.exists());
    }
}
