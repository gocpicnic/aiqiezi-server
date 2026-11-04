/*
  crates/aiqiezi-server/src/json.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::time::Instant;

use aiqiezi_proto::{error_codes, Request, Response, PROTOCOL_VERSION};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct JsonHandler {
    endpoint_id: iroh::EndpointId,
    started: Instant,
}

impl JsonHandler {
    pub fn new(endpoint: iroh::Endpoint) -> Self {
        Self {
            endpoint_id: endpoint.id(),
            started: Instant::now(),
        }
    }

    fn handle_line(&self, line: &str) -> Response {
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

        let data = match request.command.as_str() {
            "server-info" => json!({
                "name": "aiqiezi-server",
                "version": env!("CARGO_PKG_VERSION"),
                "protocol_version": PROTOCOL_VERSION,
                "endpoint_id": self.endpoint_id.to_string(),
            }),
            "status" => json!({
                "name": "aiqiezi-server",
                "version": env!("CARGO_PKG_VERSION"),
                "protocol_version": PROTOCOL_VERSION,
                "endpoint_id": self.endpoint_id.to_string(),
                "uptime_secs": self.started.elapsed().as_secs(),
            }),
            _ => {
                return Response::error(
                    request.request_id,
                    error_codes::UNKNOWN_COMMAND,
                    format!("unknown command: {}", request.command),
                );
            }
        };

        Response::success(request.request_id, data)
    }

    async fn serve(&self, conn: Connection) -> anyhow::Result<()> {
        loop {
            let (mut send, recv) = match conn.accept_bi().await {
                Ok(stream) => stream,
                Err(error) => {
                    debug!("JSON connection closed: {error}");
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
                        warn!("failed to read JSON request: {error}");
                        break;
                    }
                }

                let response = self.handle_line(&line);
                let mut encoded = serde_json::to_vec(&response)
                    .map_err(|error| anyhow::anyhow!("encode JSON response: {error}"))?;
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
}

impl ProtocolHandler for JsonHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let handler = self.clone();
        tokio::spawn(async move {
            if let Err(error) = handler.serve(conn).await {
                warn!("JSON handler error: {error}");
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::JsonHandler;
    use aiqiezi_proto::{error_codes, Response, ALPN_JSON, PROTOCOL_VERSION};
    use iroh::endpoint::presets;
    use iroh::protocol::Router;
    use iroh::{Endpoint, RelayMode};
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn handler() -> JsonHandler {
        let key = iroh::SecretKey::generate();
        JsonHandler {
            endpoint_id: key.public(),
            started: Instant::now(),
        }
    }

    fn request(command: &str) -> String {
        format!(r#"{{"version":{PROTOCOL_VERSION},"command":"{command}"}}"#)
    }

    #[test]
    fn server_info_succeeds() {
        let handler = handler();
        let response = handler.handle_line(&request("server-info"));
        assert!(response.ok);
        assert_eq!(
            response.data.unwrap()["endpoint_id"],
            handler.endpoint_id.to_string()
        );
    }

    #[test]
    fn status_succeeds() {
        let response = handler().handle_line(&request("status"));
        assert!(response.ok);
        assert!(response.data.unwrap()["uptime_secs"].is_u64());
    }

    #[test]
    fn unknown_command_is_rejected() {
        let response = handler().handle_line(&request("unknown"));
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::UNKNOWN_COMMAND);
    }

    #[test]
    fn malformed_json_is_rejected() {
        let response = handler().handle_line("not-json");
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, error_codes::INVALID_JSON);
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let response = handler().handle_line(r#"{"version":999,"command":"server-info"}"#);
        assert!(!response.ok);
        assert_eq!(
            response.error.unwrap().code,
            error_codes::UNSUPPORTED_PROTOCOL
        );
    }

    async fn test_endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .unwrap()
            .bind()
            .await
            .unwrap()
    }

    #[test]
    fn json_lines_work_over_iroh() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let server_endpoint = test_endpoint().await;
            let router = Router::builder(server_endpoint.clone())
                .accept(ALPN_JSON, JsonHandler::new(server_endpoint.clone()))
                .spawn();

            let client = test_endpoint().await;
            let response = tokio::time::timeout(Duration::from_secs(10), async {
                let conn = client
                    .connect(server_endpoint.addr(), ALPN_JSON)
                    .await
                    .unwrap();

                let (mut send, recv) = conn.open_bi().await.unwrap();
                send.write_all(request("server-info").as_bytes())
                    .await
                    .unwrap();
                send.write_all(b"\n").await.unwrap();
                send.flush().await.unwrap();

                let mut reader = BufReader::new(recv);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                serde_json::from_str::<Response>(&line).unwrap()
            })
            .await
            .expect("JSON request timed out");

            assert!(response.ok);
            assert_eq!(
                response.data.unwrap()["endpoint_id"],
                server_endpoint.id().to_string()
            );

            client.close().await;
            router.shutdown().await.unwrap();
            server_endpoint.close().await;
        });
    }
}
