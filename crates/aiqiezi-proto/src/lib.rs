/*
  crates/aiqiezi-proto/src/lib.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

//! Shared protocol types for aiqiezi clients and the server.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The wire protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// ALPN used for JSON control and metadata requests.
pub const ALPN_JSON: &[u8] = b"/aiqiezi-json/1";

pub mod error_codes {
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const INVALID_JSON: &str = "invalid_json";
    pub const UNSUPPORTED_PROTOCOL: &str = "unsupported_protocol";
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    pub const INTERNAL: &str = "internal";
}

/// One JSON Lines request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

/// One JSON Lines response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

impl Response {
    pub fn success(request_id: Option<String>, data: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(request_id: Option<String>, code: &str, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: false,
            data: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{error_codes, Request, Response, PROTOCOL_VERSION};
    use serde_json::json;

    #[test]
    fn request_round_trip() {
        let request = Request {
            version: PROTOCOL_VERSION,
            request_id: Some("req-1".to_string()),
            command: "server-info".to_string(),
            params: json!({"value": 1}),
        };

        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn success_response_round_trip() {
        let response = Response::success(Some("req-1".to_string()), json!({"ok": true}));
        let encoded = serde_json::to_string(&response).unwrap();
        let decoded: Response = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, response);
        assert!(decoded.ok);
        assert!(decoded.error.is_none());
    }

    #[test]
    fn error_response_round_trip() {
        let response = Response::error(
            Some("req-1".to_string()),
            error_codes::UNKNOWN_COMMAND,
            "unknown command",
        );
        let encoded = serde_json::to_string(&response).unwrap();
        let decoded: Response = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, response);
        assert!(!decoded.ok);
        assert_eq!(decoded.error.unwrap().code, error_codes::UNKNOWN_COMMAND);
    }

    #[test]
    fn request_params_default_to_null() {
        let request: Request = serde_json::from_str(r#"{"version":1,"command":"status"}"#).unwrap();

        assert_eq!(request.version, PROTOCOL_VERSION);
        assert_eq!(request.request_id, None);
        assert_eq!(request.params, serde_json::Value::Null);
    }
}
