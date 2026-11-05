/*
  crates/aiqiezi-server/src/firewall.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use aiqiezi_proto::ALPN_AUTH;
use iroh::endpoint::{
    AfterHandshakeOutcome, BeforeConnectOutcome, Connection, EndpointHooks, Side,
};
use tracing::{debug, warn};

use crate::auth::AuthState;

#[derive(Debug, Clone)]
pub struct AuthHooks {
    state: AuthState,
}

impl AuthHooks {
    pub fn new(state: AuthState) -> Self {
        Self { state }
    }
}

impl EndpointHooks for AuthHooks {
    async fn before_connect(
        &self,
        _remote_addr: &iroh::EndpointAddr,
        _alpn: &[u8],
    ) -> BeforeConnectOutcome {
        BeforeConnectOutcome::Accept
    }

    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        if conn.side() != Side::Server {
            return AfterHandshakeOutcome::Accept;
        }

        if conn.alpn() == ALPN_AUTH {
            return AfterHandshakeOutcome::Accept;
        }

        let remote_id = conn.remote_id();
        if self.state.is_client_allowed(&remote_id) {
            debug!("accepted authenticated connection from {remote_id}");
            AfterHandshakeOutcome::Accept
        } else {
            warn!("rejected unauthorized connection from {remote_id}");
            AfterHandshakeOutcome::Reject {
                error_code: 403u32.into(),
                reason: b"unauthorized".to_vec(),
            }
        }
    }
}
