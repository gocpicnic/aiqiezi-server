/*
  crates/aiqiezi-server/src/version.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

/// The UTC build date supplied by the Makefile.
pub const BUILD_DATE: &str = env!("AIQIEZI_BUILD_DATE");

/// The version string shown by `--version` and the JSON status commands.
pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (build ",
    env!("AIQIEZI_BUILD_DATE"),
    ")"
);
