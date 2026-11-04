/*
  crates/aiqiezi-server/src/shutdown.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use tokio::sync::watch;

/// A cloneable shutdown signal shared by the signal handler and admin server.
#[derive(Clone)]
pub struct Shutdown {
    sender: watch::Sender<bool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (sender, _receiver) = watch::channel(false);
        Self { sender }
    }

    pub fn trigger(&self) {
        self.sender.send_replace(true);
    }

    pub async fn wait(&self) {
        let mut receiver = self.sender.subscribe();
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Shutdown;

    #[tokio::test]
    async fn trigger_wakes_waiters() {
        let shutdown = Shutdown::new();
        let waiter = shutdown.clone();
        let task = tokio::spawn(async move {
            waiter.wait().await;
        });

        shutdown.trigger();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn trigger_before_wait_is_observed() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        shutdown.wait().await;
    }
}
