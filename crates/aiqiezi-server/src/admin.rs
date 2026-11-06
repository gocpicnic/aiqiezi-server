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

use crate::auth::{self, AuthState};
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

    let (response, shutdown_after_response) = handle_line(&state, &line).await;
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

async fn handle_line(state: &AdminState, line: &str) -> (Response, bool) {
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
        "set-password" => (
            handle_set_password(state, request.request_id, &request.params).await,
            false,
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

async fn handle_set_password(
    state: &AdminState,
    request_id: Option<String>,
    params: &Value,
) -> Response {
    let old_password = match string_param(params, "old_password") {
        Some(password) if password_input_is_bounded(password) => password.to_string(),
        _ => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "set-password requires a non-empty old_password within the length limit",
            );
        }
    };
    let new_password = match string_param(params, "new_password") {
        Some(password) if password_input_is_bounded(password) => password.to_string(),
        _ => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "set-password requires a non-empty new_password within the length limit",
            );
        }
    };
    let keep_clients = match params.get("keep_clients") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "keep_clients must be a boolean",
            );
        }
    };

    let verified = match state.auth.verify_password(old_password.clone()).await {
        Ok(verified) => verified,
        Err(message) => {
            return Response::error(request_id, error_codes::INTERNAL, message);
        }
    };

    if !verified {
        return Response::error(
            request_id,
            error_codes::INVALID_CREDENTIALS,
            "invalid current password",
        );
    }

    if old_password == new_password {
        return Response::error(
            request_id,
            error_codes::PASSWORD_UNCHANGED,
            "new password must differ from the current password",
        );
    }

    if let Err(message) = auth::validate_password_strength(&new_password) {
        return Response::error(request_id, error_codes::WEAK_PASSWORD, message);
    }

    let new_hash = match state.auth.hash_password(new_password).await {
        Ok(hash) => hash,
        Err(message) => {
            return Response::error(
                request_id,
                error_codes::INTERNAL,
                format!("failed to hash password: {message}"),
            );
        }
    };

    if let Err(error) = state.auth.reset_password(new_hash, keep_clients) {
        return Response::error(
            request_id,
            error_codes::INTERNAL,
            format!("failed to store new password: {error}"),
        );
    }

    state.auth.clear_all_login_failures();
    info!(keep_clients, "server password changed via admin socket");
    Response::success(
        request_id,
        json!({
            "message": if keep_clients {
                "password changed; existing clients remain authorized"
            } else {
                "password changed; all clients must authenticate again"
            },
            "password_changed": true,
            "keep_clients": keep_clients,
            "enabled_clients": state.auth.client_count(),
            "password_change_required": false,
        }),
    )
}

fn string_param<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str)
}

fn password_input_is_bounded(password: &str) -> bool {
    !password.is_empty() && password.chars().count() <= auth::PASSWORD_MAX_LEN
}

#[cfg(test)]
mod tests {
    use super::{handle_line, AdminServer, AdminState};
    use crate::auth::{verify_password_hash, AuthState, BOOTSTRAP_TEST_PASSWORD};
    use crate::shutdown::Shutdown;
    use aiqiezi_proto::{error_codes, Response, PROTOCOL_VERSION};
    use serde_json::json;
    use std::time::Instant;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const NEW_PASSWORD: &str = "NewPass-456!";

    fn state_with_dir() -> (AdminState, tempfile::TempDir) {
        let key = iroh::SecretKey::generate();
        let auth_dir = tempfile::tempdir().unwrap();
        let auth =
            AuthState::load_or_initialize(auth_dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        (
            AdminState {
                endpoint_id: key.public(),
                started: Instant::now(),
                shutdown: Shutdown::new(),
                auth,
            },
            auth_dir,
        )
    }

    fn state() -> AdminState {
        state_with_dir().0
    }

    fn request(command: &str) -> String {
        format!(r#"{{"version":{PROTOCOL_VERSION},"command":"{command}"}}"#)
    }

    fn request_with_params(command: &str, params: serde_json::Value) -> String {
        json!({
            "version": PROTOCOL_VERSION,
            "request_id": "test",
            "command": command,
            "params": params,
        })
        .to_string()
    }

    #[tokio::test]
    async fn status_returns_safe_fields() {
        let state = state();
        let (response, shutdown) = handle_line(&state, &request("status")).await;
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

    #[tokio::test]
    async fn healthcheck_returns_ok() {
        let (response, _) = handle_line(&state(), &request("healthcheck")).await;
        assert!(response.ok);
        assert_eq!(response.data.unwrap()["ok"], true);
    }

    #[tokio::test]
    async fn malformed_json_is_invalid_request() {
        let (response, _) = handle_line(&state(), "not-json").await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn unknown_command_is_invalid_request() {
        let (response, _) = handle_line(&state(), &request("unknown")).await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn reload_whitelist_requires_database() {
        let (response, _) = handle_line(&state(), &request("reload-whitelist")).await;
        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::DATABASE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn shutdown_requests_shutdown_after_response() {
        let (_response, shutdown) = handle_line(&state(), &request("shutdown")).await;
        assert!(shutdown);
    }

    #[tokio::test]
    async fn set_password_clears_clients_and_updates_hash() {
        let (state, _dir) = state_with_dir();
        let client = iroh::SecretKey::generate().public();
        state
            .auth
            .enroll_client_if_revision(client, state.auth.password_revision())
            .unwrap();
        assert_eq!(state.auth.client_count(), 1);

        let (response, shutdown) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": NEW_PASSWORD,
                }),
            ),
        )
        .await;

        assert!(response.ok, "{:?}", response.error);
        assert!(!shutdown);
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains(NEW_PASSWORD));
        assert!(!encoded.contains(BOOTSTRAP_TEST_PASSWORD));
        let data = response.data.unwrap();
        assert_eq!(data["password_changed"], true);
        assert_eq!(data["keep_clients"], false);
        assert_eq!(data["enabled_clients"], 0);
        assert_eq!(data["password_change_required"], false);
        assert!(data.get("password_hash").is_none());
        assert!(!state.auth.must_change_password());
        assert_eq!(state.auth.client_count(), 0);
        assert!(verify_password_hash(
            &state.auth.password_hash(),
            NEW_PASSWORD
        ));
        assert!(!verify_password_hash(
            &state.auth.password_hash(),
            BOOTSTRAP_TEST_PASSWORD
        ));
    }

    #[tokio::test]
    async fn set_password_can_keep_clients() {
        let (state, _dir) = state_with_dir();
        let client = iroh::SecretKey::generate().public();
        state
            .auth
            .enroll_client_if_revision(client, state.auth.password_revision())
            .unwrap();

        let (response, _) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": NEW_PASSWORD,
                    "keep_clients": true,
                }),
            ),
        )
        .await;

        assert!(response.ok, "{:?}", response.error);
        assert_eq!(response.data.unwrap()["enabled_clients"], 1);
        assert!(state.auth.is_client_allowed(&client));
        assert_eq!(state.auth.client_count(), 1);
    }

    #[tokio::test]
    async fn set_password_rejects_wrong_current_password() {
        let (state, _dir) = state_with_dir();

        let (response, _) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({
                    "old_password": "WrongPass-123!",
                    "new_password": NEW_PASSWORD,
                }),
            ),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::INVALID_CREDENTIALS
        );
        assert!(verify_password_hash(
            &state.auth.password_hash(),
            BOOTSTRAP_TEST_PASSWORD
        ));
    }

    #[tokio::test]
    async fn set_password_rejects_unchanged_password() {
        let (state, _dir) = state_with_dir();

        let (response, _) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": BOOTSTRAP_TEST_PASSWORD,
                }),
            ),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::PASSWORD_UNCHANGED
        );
    }

    #[tokio::test]
    async fn set_password_rejects_weak_password_and_missing_params() {
        let (state, _dir) = state_with_dir();

        let (response, _) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": "short",
                }),
            ),
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::WEAK_PASSWORD);

        let (response, _) = handle_line(
            &state,
            &request_with_params(
                "set-password",
                json!({"old_password": BOOTSTRAP_TEST_PASSWORD}),
            ),
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
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
