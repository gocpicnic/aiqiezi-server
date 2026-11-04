/*
  crates/aiqiezi-server/src/error.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use thiserror::Error;

use crate::config::ConfigError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("logging initialization failed: {0}")]
    Logging(String),

    #[error("runtime error: {0:#}")]
    Runtime(#[from] anyhow::Error),
}
