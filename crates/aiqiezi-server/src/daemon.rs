/*
  crates/aiqiezi-server/src/daemon.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use tracing::{info, warn};

use crate::admin::{AdminServer, AdminState};
use crate::config::Config;
use crate::endpoint;
use crate::lock::InstanceLock;
use crate::shutdown::Shutdown;

/// Run the single daemon process with the iroh and admin listeners.
pub async fn run(config: &Config) -> anyhow::Result<()> {
    let configured_lock_path = config.server_lock_path();
    let lock = InstanceLock::acquire(&config.runtime_dir)?;
    debug_assert_eq!(lock.path(), configured_lock_path);
    info!(
        lock = %configured_lock_path.display(),
        "instance lock acquired"
    );

    let shutdown = Shutdown::new();
    let running_endpoint = endpoint::start(config).await?;
    let admin = match AdminServer::bind(
        config.admin_socket_path(),
        AdminState::new(
            running_endpoint.id(),
            shutdown.clone(),
            running_endpoint.auth(),
        ),
    ) {
        Ok(admin) => admin,
        Err(error) => {
            running_endpoint.shutdown().await?;
            return Err(error);
        }
    };

    info!(
        socket = %config.admin_socket_path().display(),
        "admin socket listening"
    );
    info!(
        data_dir = %config.data_dir.display(),
        "aiqiezi-server is running"
    );

    let mut admin_task = tokio::spawn(admin.serve());
    let mut signal_task = tokio::spawn(wait_for_signal(shutdown.clone()));
    let mut admin_finished = false;

    tokio::select! {
        result = &mut admin_task => {
            log_admin_result(result);
            admin_finished = true;
        }
        _ = &mut signal_task => {}
    }

    shutdown.trigger();
    signal_task.abort();

    if !admin_finished {
        if let Err(error) = admin_task.await {
            warn!("admin task failed during shutdown: {error}");
        }
    }

    running_endpoint.shutdown().await?;
    info!("shutdown complete");
    drop(lock);
    Ok(())
}

fn log_admin_result(result: Result<anyhow::Result<()>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => info!("admin socket stopped"),
        Ok(Err(error)) => warn!("admin socket failed: {error}"),
        Err(error) => warn!("admin task failed: {error}"),
    }
}

async fn wait_for_signal(shutdown: Shutdown) {
    #[cfg(unix)]
    let signal = {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => "Ctrl+C",
            _ = terminate.recv() => "SIGTERM",
        }
    };

    #[cfg(not(unix))]
    let signal = {
        tokio::signal::ctrl_c().await.ok();
        "Ctrl+C"
    };

    info!("received {signal}, shutting down");
    shutdown.trigger();
}
