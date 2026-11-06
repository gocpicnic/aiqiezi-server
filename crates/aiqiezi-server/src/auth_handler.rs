/*
  crates/aiqiezi-server/src/auth_handler.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use aiqiezi_proto::{error_codes, Request, Response, PROTOCOL_VERSION};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::EndpointId;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn};

use crate::auth::{self, AuthState};

#[derive(Debug, Clone)]
pub struct AuthHandler {
    state: AuthState,
}

impl AuthHandler {
    pub fn new(state: AuthState) -> Self {
        Self { state }
    }
}

impl ProtocolHandler for AuthHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let state = self.state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_connection(conn, state).await {
                warn!("auth connection error: {error}");
            }
        });
        Ok(())
    }
}

async fn handle_connection(conn: Connection, state: AuthState) -> anyhow::Result<()> {
    let remote_id = conn.remote_id();

    loop {
        let (mut send, recv) = match conn.accept_bi().await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::debug!("auth connection closed: {error}");
                break;
            }
        };

        let mut reader = BufReader::new(recv);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => {
                    warn!("failed to read auth request: {error}");
                    break;
                }
            }

            if line.trim().is_empty() {
                continue;
            }

            let response = handle_line(&state, remote_id, &line).await;
            let mut encoded = serde_json::to_vec(&response)
                .map_err(|error| anyhow::anyhow!("encode auth response: {error}"))?;
            encoded.push(b'\n');

            if send.write_all(&encoded).await.is_err() {
                break;
            }
            if send.flush().await.is_err() {
                break;
            }
        }
    }

    Ok(())
}

async fn handle_line(state: &AuthState, remote_id: EndpointId, line: &str) -> Response {
    let request = match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => request,
        Err(error) => {
            return Response::error(
                None,
                error_codes::INVALID_JSON,
                format!("invalid JSON request: {error}"),
            );
        }
    };

    if request.version != PROTOCOL_VERSION {
        return Response::error(
            request.request_id,
            error_codes::UNSUPPORTED_PROTOCOL,
            format!(
                "unsupported protocol version {}, expected {}",
                request.version, PROTOCOL_VERSION
            ),
        );
    }

    let request_id = request.request_id;
    match request.command.as_str() {
        "login" => handle_login(state, remote_id, request_id, &request.params).await,
        "change-password" => {
            handle_change_password(state, remote_id, request_id, &request.params).await
        }
        _ => Response::error(
            request_id,
            error_codes::UNKNOWN_COMMAND,
            format!("unknown auth command: {}", request.command),
        ),
    }
}

async fn handle_login(
    state: &AuthState,
    remote_id: EndpointId,
    request_id: Option<String>,
    params: &Value,
) -> Response {
    let password = match string_param(params, "password") {
        Some(password) if password_input_is_bounded(password) => password.to_string(),
        _ => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "login requires a non-empty password within the length limit",
            );
        }
    };

    if let Some(retry_after) = state.login_retry_after(&remote_id) {
        return Response::error(
            request_id,
            error_codes::RATE_LIMITED,
            format!("too many failed login attempts; retry in {retry_after}s"),
        );
    }

    let password_revision = state.password_revision();
    let verified = match state.verify_password(password).await {
        Ok(verified) => verified,
        Err(message) => {
            return Response::error(request_id, error_codes::INTERNAL, message);
        }
    };

    if !verified {
        state.record_login_failure(remote_id);
        return Response::error(
            request_id,
            error_codes::INVALID_CREDENTIALS,
            "invalid password",
        );
    }

    state.clear_login_failures(&remote_id);

    if state.must_change_password() {
        return Response::error(
            request_id,
            error_codes::PASSWORD_CHANGE_REQUIRED,
            "change the bootstrap password before using the server",
        );
    }

    match state.enroll_client_if_revision(remote_id, password_revision) {
        Ok(true) => {}
        Ok(false) => {
            return Response::error(
                request_id,
                error_codes::INVALID_CREDENTIALS,
                "password changed during authentication; log in again",
            );
        }
        Err(error) => {
            return Response::error(
                request_id,
                error_codes::INTERNAL,
                format!("failed to enroll client: {error}"),
            );
        }
    }

    info!("client authenticated: {remote_id}");
    Response::success(
        request_id,
        json!({
            "authenticated": true,
            "endpoint_id": remote_id.to_string(),
            "password_change_required": false,
        }),
    )
}

async fn handle_change_password(
    state: &AuthState,
    remote_id: EndpointId,
    request_id: Option<String>,
    params: &Value,
) -> Response {
    let old_password = match string_param(params, "old_password") {
        Some(password) if password_input_is_bounded(password) => password.to_string(),
        _ => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "change-password requires a non-empty old_password within the length limit",
            );
        }
    };
    let new_password = match string_param(params, "new_password") {
        Some(password) if password_input_is_bounded(password) => password.to_string(),
        _ => {
            return Response::error(
                request_id,
                error_codes::INVALID_REQUEST,
                "change-password requires a non-empty new_password within the length limit",
            );
        }
    };

    if let Some(retry_after) = state.login_retry_after(&remote_id) {
        return Response::error(
            request_id,
            error_codes::RATE_LIMITED,
            format!("too many failed authentication attempts; retry in {retry_after}s"),
        );
    }

    let verified = match state.verify_password(old_password.clone()).await {
        Ok(verified) => verified,
        Err(message) => {
            return Response::error(request_id, error_codes::INTERNAL, message);
        }
    };

    if !verified {
        state.record_login_failure(remote_id);
        return Response::error(
            request_id,
            error_codes::INVALID_CREDENTIALS,
            "invalid old password",
        );
    }

    if old_password == new_password {
        return Response::error(
            request_id,
            error_codes::PASSWORD_UNCHANGED,
            "new password must differ from the old password",
        );
    }

    if let Err(message) = auth::validate_password_strength(&new_password) {
        return Response::error(request_id, error_codes::WEAK_PASSWORD, message);
    }

    let new_hash = match state.hash_password(new_password).await {
        Ok(hash) => hash,
        Err(message) => {
            return Response::error(
                request_id,
                error_codes::INTERNAL,
                format!("failed to hash password: {message}"),
            );
        }
    };

    if let Err(error) = state.change_password(new_hash, remote_id) {
        return Response::error(
            request_id,
            error_codes::INTERNAL,
            format!("failed to store new password: {error}"),
        );
    }

    state.clear_login_failures(&remote_id);
    info!("password changed and client enrolled: {remote_id}");
    Response::success(
        request_id,
        json!({
            "authenticated": true,
            "endpoint_id": remote_id.to_string(),
            "password_changed": true,
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
    use super::handle_line;
    use crate::auth::{self, AuthState, BOOTSTRAP_TEST_PASSWORD};
    use aiqiezi_proto::{error_codes, PROTOCOL_VERSION};
    use iroh::SecretKey;
    use serde_json::json;

    fn request(command: &str, params: serde_json::Value) -> String {
        json!({
            "version": PROTOCOL_VERSION,
            "request_id": "test",
            "command": command,
            "params": params,
        })
        .to_string()
    }

    fn endpoint_id() -> iroh::EndpointId {
        SecretKey::generate().public()
    }

    #[tokio::test]
    async fn bootstrap_login_requires_password_change() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();

        let response = handle_line(
            &state,
            caller,
            &request("login", json!({"password": BOOTSTRAP_TEST_PASSWORD})),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::PASSWORD_CHANGE_REQUIRED
        );
        assert!(!state.is_client_allowed(&caller));
    }

    #[tokio::test]
    async fn change_password_enrolls_and_allows_login() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();
        let new_password = "NewPass-456!";

        let response = handle_line(
            &state,
            caller,
            &request(
                "change-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": new_password,
                }),
            ),
        )
        .await;
        assert!(response.ok, "{:?}", response.error);
        assert!(state.is_client_allowed(&caller));
        assert!(!state.must_change_password());

        let response = handle_line(
            &state,
            caller,
            &request("login", json!({"password": new_password})),
        )
        .await;
        assert!(response.ok, "{:?}", response.error);
        assert_eq!(response.data.unwrap()["authenticated"], true)
    }

    #[tokio::test]
    async fn wrong_password_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();

        let response = handle_line(
            &state,
            caller,
            &request("login", json!({"password": "WrongPass-123!"})),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::INVALID_CREDENTIALS
        );
    }

    #[tokio::test]
    async fn oversized_password_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();
        let password = "A".repeat(auth::PASSWORD_MAX_LEN + 1);

        let response = handle_line(
            &state,
            caller,
            &request("login", json!({"password": password})),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn weak_new_password_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();

        let response = handle_line(
            &state,
            caller,
            &request(
                "change-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": "short",
                }),
            ),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::WEAK_PASSWORD);
    }

    #[tokio::test]
    async fn failed_logins_are_rate_limited() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP_TEST_PASSWORD)).unwrap();
        let caller = endpoint_id();

        for _ in 0..auth::MAX_LOGIN_FAILURES {
            let _ = handle_line(
                &state,
                caller,
                &request("login", json!({"password": "WrongPass-123!"})),
            )
            .await;
        }

        let response = handle_line(
            &state,
            caller,
            &request("login", json!({"password": BOOTSTRAP_TEST_PASSWORD})),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::RATE_LIMITED);
    }
}
