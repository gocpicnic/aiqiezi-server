/*
  crates/aiqiezi-server/src/config.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use thiserror::Error;

/// Runtime configuration loaded from environment variables.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub data_dir: PathBuf,
    pub keys_dir: PathBuf,
    pub media_dir: PathBuf,
    pub config_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub database_url: String,
    pub auto_migrate: bool,
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
    pub dns_server: Option<String>,
    pub bind_addr: Option<SocketAddr>,
    pub log_level: String,
    pub bootstrap_password: Option<String>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("keys_dir", &self.keys_dir)
            .field("media_dir", &self.media_dir)
            .field("config_dir", &self.config_dir)
            .field("runtime_dir", &self.runtime_dir)
            .field("database_url", &"<redacted>")
            .field("auto_migrate", &self.auto_migrate)
            .field("relay_url", &self.relay_url)
            .field("relay_token", &"<redacted>")
            .field("dns_server", &self.dns_server)
            .field("bind_addr", &self.bind_addr)
            .field("log_level", &self.log_level)
            .field(
                "bootstrap_password",
                &self.bootstrap_password.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),

    #[error("invalid value for {name}: {value}")]
    InvalidBool { name: &'static str, value: String },

    #[error("invalid value for AIQIEZI_BIND_ADDR: {0}")]
    InvalidBindAddr(String),

    #[error("AIQIEZI_DATABASE_URL must start with postgres:// or postgresql://")]
    InvalidDatabaseUrl,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_iter(std::env::vars())
    }

    pub fn from_iter<I, K, V>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars: HashMap<String, String> = vars
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();

        let data_dir = optional_nonempty(&vars, "AIQIEZI_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./data"));

        let database_url = required_nonempty(&vars, "AIQIEZI_DATABASE_URL")?;
        if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
            return Err(ConfigError::InvalidDatabaseUrl);
        }

        let auto_migrate = match optional_nonempty(&vars, "AIQIEZI_AUTO_MIGRATE") {
            Some(value) => parse_bool("AIQIEZI_AUTO_MIGRATE", &value)?,
            None => false,
        };

        let bind_addr = match optional_nonempty(&vars, "AIQIEZI_BIND_ADDR") {
            Some(value) => Some(
                value
                    .parse::<SocketAddr>()
                    .map_err(|_| ConfigError::InvalidBindAddr(value))?,
            ),
            None => None,
        };

        let runtime_dir = optional_nonempty(&vars, "AIQIEZI_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/aiqiezi"));

        let log_level =
            optional_nonempty(&vars, "AIQIEZI_LOG_LEVEL").unwrap_or_else(|| "info".to_string());

        Ok(Self {
            keys_dir: data_dir.join("keys"),
            media_dir: data_dir.join("media"),
            config_dir: data_dir.join("config"),
            runtime_dir,
            data_dir,
            database_url,
            auto_migrate,
            relay_url: optional_nonempty(&vars, "AIQIEZI_RELAY_URL"),
            relay_token: optional_nonempty(&vars, "AIQIEZI_RELAY_TOKEN"),
            dns_server: optional_nonempty(&vars, "AIQIEZI_DNS_SERVER"),
            bind_addr,
            log_level,
            bootstrap_password: optional_nonempty(&vars, "AIQIEZI_BOOTSTRAP_PASSWORD"),
        })
    }

    pub fn admin_socket_path(&self) -> PathBuf {
        self.runtime_dir.join("admin.sock")
    }

    pub fn server_lock_path(&self) -> PathBuf {
        self.runtime_dir.join("server.lock")
    }
}

fn optional_nonempty(vars: &HashMap<String, String>, name: &str) -> Option<String> {
    vars.get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn required_nonempty(
    vars: &HashMap<String, String>,
    name: &'static str,
) -> Result<String, ConfigError> {
    optional_nonempty(vars, name).ok_or(ConfigError::Missing(name))
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidBool {
            name,
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::path::PathBuf;

    fn base_vars() -> Vec<(String, String)> {
        vec![
            (
                "AIQIEZI_DATABASE_URL".to_string(),
                "postgres://user:pass@localhost/aiqiezi".to_string(),
            ),
            (
                "AIQIEZI_DATA_DIR".to_string(),
                "/tmp/aiqiezi-test".to_string(),
            ),
        ]
    }

    #[test]
    fn defaults_are_safe() {
        let config = Config::from_iter(base_vars()).unwrap();

        assert_eq!(config.data_dir, PathBuf::from("/tmp/aiqiezi-test"));
        assert_eq!(config.keys_dir, PathBuf::from("/tmp/aiqiezi-test/keys"));
        assert_eq!(config.media_dir, PathBuf::from("/tmp/aiqiezi-test/media"));
        assert_eq!(config.config_dir, PathBuf::from("/tmp/aiqiezi-test/config"));
        assert_eq!(config.runtime_dir, PathBuf::from("/run/aiqiezi"));
        assert_eq!(
            config.admin_socket_path(),
            PathBuf::from("/run/aiqiezi/admin.sock")
        );
        assert_eq!(
            config.server_lock_path(),
            PathBuf::from("/run/aiqiezi/server.lock")
        );
        assert_eq!(
            config.database_url,
            "postgres://user:pass@localhost/aiqiezi"
        );
        assert!(!config.auto_migrate);
        assert_eq!(config.relay_url, None);
        assert_eq!(config.relay_token, None);
        assert_eq!(config.dns_server, None);
        assert_eq!(config.bind_addr, None);
        assert_eq!(config.log_level, "info");
        assert_eq!(config.bootstrap_password, None);
    }

    #[test]
    fn missing_database_url_is_an_error() {
        let error = Config::from_iter(std::iter::empty::<(&str, &str)>()).unwrap_err();
        assert_eq!(error, ConfigError::Missing("AIQIEZI_DATABASE_URL"));
    }

    #[test]
    fn invalid_database_url_is_an_error() {
        let error = Config::from_iter([("AIQIEZI_DATABASE_URL", "not-a-database")]).unwrap_err();
        assert_eq!(error, ConfigError::InvalidDatabaseUrl);
    }

    #[test]
    fn parses_optional_settings() {
        let mut vars = base_vars();
        vars.extend([
            ("AIQIEZI_AUTO_MIGRATE".to_string(), "yes".to_string()),
            (
                "AIQIEZI_BIND_ADDR".to_string(),
                "127.0.0.1:9000".to_string(),
            ),
            (
                "AIQIEZI_RELAY_URL".to_string(),
                "https://relay.example".to_string(),
            ),
            ("AIQIEZI_RELAY_TOKEN".to_string(), "secret".to_string()),
            ("AIQIEZI_DNS_SERVER".to_string(), "dns.example".to_string()),
            (
                "AIQIEZI_RUNTIME_DIR".to_string(),
                "/tmp/aiqiezi-runtime".to_string(),
            ),
            ("AIQIEZI_LOG_LEVEL".to_string(), "debug".to_string()),
            (
                "AIQIEZI_BOOTSTRAP_PASSWORD".to_string(),
                "ChangeMe-123!".to_string(),
            ),
        ]);

        let config = Config::from_iter(vars).unwrap();

        assert!(config.auto_migrate);
        assert_eq!(
            config.bind_addr,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 9000)))
        );
        assert_eq!(config.relay_url.as_deref(), Some("https://relay.example"));
        assert_eq!(config.relay_token.as_deref(), Some("secret"));
        assert_eq!(config.dns_server.as_deref(), Some("dns.example"));
        assert_eq!(config.runtime_dir, PathBuf::from("/tmp/aiqiezi-runtime"));
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.bootstrap_password.as_deref(), Some("ChangeMe-123!"));
    }

    #[test]
    fn empty_optional_values_are_none() {
        let mut vars = base_vars();
        vars.extend([
            ("AIQIEZI_RELAY_URL".to_string(), "".to_string()),
            ("AIQIEZI_RELAY_TOKEN".to_string(), " ".to_string()),
            ("AIQIEZI_DNS_SERVER".to_string(), "".to_string()),
            ("AIQIEZI_BIND_ADDR".to_string(), "".to_string()),
            ("AIQIEZI_RUNTIME_DIR".to_string(), "".to_string()),
            ("AIQIEZI_LOG_LEVEL".to_string(), "".to_string()),
            ("AIQIEZI_BOOTSTRAP_PASSWORD".to_string(), "".to_string()),
        ]);

        let config = Config::from_iter(vars).unwrap();

        assert_eq!(config.relay_url, None);
        assert_eq!(config.relay_token, None);
        assert_eq!(config.dns_server, None);
        assert_eq!(config.bind_addr, None);
        assert_eq!(config.runtime_dir, PathBuf::from("/run/aiqiezi"));
        assert_eq!(config.log_level, "info");
        assert_eq!(config.bootstrap_password, None);
    }

    #[test]
    fn debug_redacts_secrets() {
        let mut vars = base_vars();
        vars.extend([
            (
                "AIQIEZI_BOOTSTRAP_PASSWORD".to_string(),
                "ChangeMe-123!".to_string(),
            ),
            (
                "AIQIEZI_RELAY_TOKEN".to_string(),
                "relay-secret".to_string(),
            ),
        ]);
        let config = Config::from_iter(vars).unwrap();

        let debug = format!("{config:?}");
        assert!(!debug.contains("ChangeMe-123!"));
        assert!(!debug.contains("relay-secret"));
        assert!(!debug.contains("postgres://user:pass"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn invalid_bind_addr_is_an_error() {
        let mut vars = base_vars();
        vars.push((
            "AIQIEZI_BIND_ADDR".to_string(),
            "not-an-address".to_string(),
        ));

        let error = Config::from_iter(vars).unwrap_err();
        assert_eq!(
            error,
            ConfigError::InvalidBindAddr("not-an-address".to_string())
        );
    }

    #[test]
    fn invalid_bool_is_an_error() {
        let mut vars = base_vars();
        vars.push(("AIQIEZI_AUTO_MIGRATE".to_string(), "sometimes".to_string()));

        let error = Config::from_iter(vars).unwrap_err();
        assert_eq!(
            error,
            ConfigError::InvalidBool {
                name: "AIQIEZI_AUTO_MIGRATE",
                value: "sometimes".to_string()
            }
        );
    }
}
