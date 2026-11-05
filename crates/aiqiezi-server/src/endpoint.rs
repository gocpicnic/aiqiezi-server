/*
  crates/aiqiezi-server/src/endpoint.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::path::Path;

use aiqiezi_proto::{ALPN_AUTH, ALPN_JSON};
use anyhow::Context;
use iroh::address_lookup::{DnsAddressLookup, PkarrPublisher, PkarrResolver};
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMap, RelayMode, SecretKey};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use iroh_tickets::endpoint::EndpointTicket;
use tracing::info;
use url::Url;

use crate::auth::AuthState;
use crate::auth_handler::AuthHandler;
use crate::config::Config;
use crate::firewall::AuthHooks;
use crate::json::JsonHandler;

/// Load the persistent endpoint key, creating it on first start.
pub fn load_or_create_key(config: &Config) -> anyhow::Result<SecretKey> {
    std::fs::create_dir_all(&config.keys_dir)
        .with_context(|| format!("create keys directory {}", config.keys_dir.display()))?;

    let key_path = config.keys_dir.join("secret_key.bin");
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)
            .with_context(|| format!("read endpoint key {}", key_path.display()))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "endpoint key {} has invalid length: expected 32 bytes, got {}",
                key_path.display(),
                bytes.len()
            )
        })?;
        set_private_permissions(&key_path)?;
        return Ok(SecretKey::from_bytes(&bytes));
    }

    let key = SecretKey::generate();
    std::fs::write(&key_path, key.to_bytes())
        .with_context(|| format!("write endpoint key {}", key_path.display()))?;
    set_private_permissions(&key_path)?;
    Ok(key)
}

/// Save the endpoint ticket in a stable location for client onboarding.
pub fn save_endpoint_ticket(config: &Config, endpoint: &Endpoint) -> anyhow::Result<()> {
    std::fs::create_dir_all(&config.keys_dir)
        .with_context(|| format!("create keys directory {}", config.keys_dir.display()))?;

    let ticket_path = config.keys_dir.join("endpoint_ticket.txt");
    let ticket = EndpointTicket::new(endpoint.addr());
    std::fs::write(&ticket_path, ticket.to_string())
        .with_context(|| format!("write endpoint ticket {}", ticket_path.display()))?;
    set_private_permissions(&ticket_path)?;
    Ok(())
}

fn set_private_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}

/// Resolve the relay mode from config.
fn relay_mode(config: &Config) -> RelayMode {
    let Some(url) = config.relay_url.as_deref() else {
        return RelayMode::Default;
    };

    match RelayMap::try_from_iter([url]) {
        Ok(map) => {
            let map = match config.relay_token.as_deref() {
                Some(token) => map.with_auth_token(token),
                None => map,
            };
            RelayMode::Custom(map)
        }
        Err(error) => {
            tracing::warn!("invalid relay URL {url}: {error}; using default relays");
            RelayMode::Default
        }
    }
}

/// Configure mDNS plus the default n0 DNS services or a custom DNS server.
fn apply_address_lookup(
    builder: iroh::endpoint::Builder,
    config: &Config,
) -> iroh::endpoint::Builder {
    let builder = builder
        .clear_address_lookup()
        .address_lookup(MdnsAddressLookup::builder());

    match dns_server_config(config) {
        Some((pkarr_url, origin)) => {
            info!("discovery: custom dns server pkarr={pkarr_url} origin={origin}");
            builder
                .address_lookup(PkarrPublisher::builder(pkarr_url.clone()))
                .address_lookup(PkarrResolver::builder(pkarr_url))
                .address_lookup(DnsAddressLookup::builder(origin))
        }
        None => builder
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(PkarrResolver::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns()),
    }
}

fn dns_server_config(config: &Config) -> Option<(Url, String)> {
    let raw = config.dns_server.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }

    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut url: Url = match candidate.parse() {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!("invalid DNS server {raw}: {error}; using default discovery");
            return None;
        }
    };
    if matches!(url.path().trim_matches('/'), "" | "pkarr") {
        url.set_path("/pkarr");
    }
    let Some(host) = url
        .host_str()
        .map(|host| host.trim_end_matches('.').to_string())
    else {
        tracing::warn!("invalid DNS server {raw}: missing host; using default discovery");
        return None;
    };
    if host.is_empty() || url.set_host(Some(&host)).is_err() {
        tracing::warn!("invalid DNS server {raw}: invalid host; using default discovery");
        return None;
    }

    Some((url, format!("{host}.")))
}

async fn bind_endpoint(
    config: &Config,
    key: SecretKey,
    hooks: AuthHooks,
) -> anyhow::Result<Endpoint> {
    let mut builder = apply_address_lookup(
        Endpoint::builder(presets::N0)
            .relay_mode(relay_mode(config))
            .secret_key(key)
            .hooks(hooks),
        config,
    );

    if let Some(bind_addr) = config.bind_addr {
        builder = builder
            .bind_addr(bind_addr)
            .with_context(|| format!("invalid bind address {bind_addr}"))?;
    }

    builder.bind().await.context("bind iroh endpoint")
}

pub struct RunningEndpoint {
    endpoint: Endpoint,
    router: iroh::protocol::Router,
    auth: AuthState,
}

impl RunningEndpoint {
    pub fn id(&self) -> iroh::EndpointId {
        self.endpoint.id()
    }

    pub fn auth(&self) -> AuthState {
        self.auth.clone()
    }

    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.router
            .shutdown()
            .await
            .context("shut down iroh router")?;
        self.endpoint.close().await;
        Ok(())
    }
}

/// Start the iroh endpoint, auth handler, and JSON handler.
pub async fn start(config: &Config) -> anyhow::Result<RunningEndpoint> {
    let auth =
        AuthState::load_or_initialize(&config.keys_dir, config.bootstrap_password.as_deref())?;
    let key = load_or_create_key(config)?;
    let endpoint = bind_endpoint(config, key, AuthHooks::new(auth.clone())).await?;
    info!("NODE ID: {}", endpoint.id());

    save_endpoint_ticket(config, &endpoint)?;
    info!("endpoint ticket saved");

    let handler = JsonHandler::new(endpoint.clone());
    let auth_handler = AuthHandler::new(auth.clone());
    let router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(ALPN_AUTH, auth_handler)
        .accept(ALPN_JSON, handler)
        .spawn();

    info!(
        "listening on ALPNs {:?} and {:?}",
        String::from_utf8_lossy(ALPN_AUTH),
        String::from_utf8_lossy(ALPN_JSON)
    );
    Ok(RunningEndpoint {
        endpoint,
        router,
        auth,
    })
}

#[cfg(test)]
mod tests {
    use super::{load_or_create_key, save_endpoint_ticket};
    use crate::auth::{AuthState, BOOTSTRAP_TEST_PASSWORD};
    use crate::auth_handler::AuthHandler;
    use crate::config::Config;
    use crate::firewall::AuthHooks;
    use crate::json::JsonHandler;
    use aiqiezi_proto::{error_codes, Response, ALPN_AUTH, ALPN_JSON, PROTOCOL_VERSION};
    use iroh::endpoint::{presets, Connection};
    use iroh::protocol::Router;
    use iroh::{Endpoint, RelayMode};
    use serde_json::{json, Value};
    use std::net::SocketAddr;
    use std::path::Path;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn test_endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .unwrap()
            .bind()
            .await
            .unwrap()
    }

    async fn request_succeeds(conn: &Connection, command: &str) -> bool {
        let Ok((mut send, recv)) = conn.open_bi().await else {
            return false;
        };
        let request = json!({
            "version": PROTOCOL_VERSION,
            "request_id": "test",
            "command": command,
            "params": {},
        });
        if send
            .write_all(request.to_string().as_bytes())
            .await
            .is_err()
        {
            return false;
        }
        if send.write_all(b"\n").await.is_err() {
            return false;
        }
        if send.flush().await.is_err() {
            return false;
        }

        let mut reader = BufReader::new(recv);
        let mut line = String::new();
        let Ok(bytes) = reader.read_line(&mut line).await else {
            return false;
        };
        if bytes == 0 {
            return false;
        }
        serde_json::from_str::<Response>(&line)
            .map(|response| response.ok)
            .unwrap_or(false)
    }

    async fn request(conn: &Connection, command: &str, params: Value) -> Response {
        let (mut send, recv) = conn.open_bi().await.unwrap();
        let request = json!({
            "version": PROTOCOL_VERSION,
            "request_id": "test",
            "command": command,
            "params": params,
        });
        send.write_all(request.to_string().as_bytes())
            .await
            .unwrap();
        send.write_all(b"\n").await.unwrap();
        send.flush().await.unwrap();

        let mut reader = BufReader::new(recv);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn firewall_requires_auth_before_json() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let auth_dir = tempfile::tempdir().unwrap();
            let auth = AuthState::load_or_initialize(
                &auth_dir.path().join("keys"),
                Some(BOOTSTRAP_TEST_PASSWORD),
            )
            .unwrap();

            let server_endpoint = Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .hooks(AuthHooks::new(auth.clone()))
                .bind()
                .await
                .unwrap();
            let router = Router::builder(server_endpoint.clone())
                .accept(ALPN_AUTH, AuthHandler::new(auth.clone()))
                .accept(ALPN_JSON, JsonHandler::new(server_endpoint.clone()))
                .spawn();

            let client = test_endpoint().await;

            let unauthorized = tokio::time::timeout(
                Duration::from_secs(10),
                client.connect(server_endpoint.addr(), ALPN_JSON),
            )
            .await
            .expect("unauthorized JSON connection timed out");
            let blocked = match unauthorized {
                Err(_) => true,
                Ok(conn) => !request_succeeds(&conn, "server-info").await,
            };
            assert!(blocked, "unauthorized JSON request was not blocked");

            let auth_conn = tokio::time::timeout(
                Duration::from_secs(10),
                client.connect(server_endpoint.addr(), ALPN_AUTH),
            )
            .await
            .expect("auth connection timed out")
            .unwrap();

            let response = request(
                &auth_conn,
                "login",
                json!({"password": BOOTSTRAP_TEST_PASSWORD}),
            )
            .await;
            assert!(!response.ok);
            assert_eq!(
                response.error.unwrap().code,
                error_codes::PASSWORD_CHANGE_REQUIRED
            );

            let new_password = "NewPass-456!";
            let response = request(
                &auth_conn,
                "change-password",
                json!({
                    "old_password": BOOTSTRAP_TEST_PASSWORD,
                    "new_password": new_password,
                }),
            )
            .await;
            assert!(response.ok, "{:?}", response.error);

            let json_conn = tokio::time::timeout(
                Duration::from_secs(10),
                client.connect(server_endpoint.addr(), ALPN_JSON),
            )
            .await
            .expect("authorized JSON connection timed out")
            .unwrap();
            let response = request(&json_conn, "server-info", json!({})).await;
            assert!(response.ok, "{:?}", response.error);

            json_conn.close(0u32.into(), b"test complete");
            auth_conn.close(0u32.into(), b"test complete");
            client.close().await;
            router.shutdown().await.unwrap();
            server_endpoint.close().await;
        });
    }

    fn config(data_dir: &Path) -> Config {
        Config {
            data_dir: data_dir.to_path_buf(),
            keys_dir: data_dir.join("keys"),
            media_dir: data_dir.join("media"),
            config_dir: data_dir.join("config"),
            runtime_dir: data_dir.join("run"),
            database_url: "postgres://test:test@127.0.0.1/test".to_string(),
            auto_migrate: false,
            relay_url: None,
            relay_token: None,
            dns_server: None,
            bind_addr: None,
            log_level: "info".to_string(),
            bootstrap_password: None,
        }
    }

    #[test]
    fn same_key_produces_same_endpoint_id() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());

        let first = load_or_create_key(&config).unwrap();
        let second = load_or_create_key(&config).unwrap();

        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn malformed_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        std::fs::create_dir_all(&config.keys_dir).unwrap();
        std::fs::write(config.keys_dir.join("secret_key.bin"), [0u8; 16]).unwrap();

        assert!(load_or_create_key(&config).is_err());
    }

    #[test]
    fn ticket_is_updated_after_endpoint_bind() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        std::fs::create_dir_all(&config.keys_dir).unwrap();
        let ticket_path = config.keys_dir.join("endpoint_ticket.txt");
        std::fs::write(&ticket_path, "stale").unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let endpoint = Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .bind()
                .await
                .unwrap();

            save_endpoint_ticket(&config, &endpoint).unwrap();
            endpoint.close().await;
        });

        let ticket = std::fs::read_to_string(&ticket_path).unwrap();
        assert_ne!(ticket, "stale");
        assert!(ticket.starts_with("endpoint"));
    }
}
