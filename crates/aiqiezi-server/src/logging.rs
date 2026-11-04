/*
  crates/aiqiezi-server/src/logging.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;

/// Initialize structured logging to stderr.
pub fn init(filter: &str) -> Result<(), String> {
    let filter = EnvFilter::try_new(filter)
        .map_err(|error| format!("invalid log filter {filter:?}: {error}"))?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init()
        .map_err(|error| format!("failed to install tracing subscriber: {error}"))
}
