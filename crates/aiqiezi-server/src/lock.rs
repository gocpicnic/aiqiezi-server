/*
  crates/aiqiezi-server/src/lock.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// An exclusive advisory lock that prevents two daemon instances from running.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn acquire(runtime_dir: &Path) -> anyhow::Result<Self> {
        create_runtime_dir(runtime_dir)?;

        let path = runtime_dir.join("server.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open instance lock {}", path.display()))?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!(
                    "another aiqiezi-server instance holds the lock {}",
                    path.display()
                );
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("lock instance file {}", path.display()));
            }
        }

        file.set_len(0)
            .with_context(|| format!("truncate instance lock {}", path.display()))?;
        (&file)
            .seek(SeekFrom::Start(0))
            .with_context(|| format!("seek instance lock {}", path.display()))?;
        writeln!(&file, "{}", std::process::id())
            .with_context(|| format!("write instance lock {}", path.display()))?;
        (&file)
            .flush()
            .with_context(|| format!("flush instance lock {}", path.display()))?;

        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Create the runtime directory with restrictive permissions.
pub fn create_runtime_dir(runtime_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(runtime_dir)
        .with_context(|| format!("create runtime directory {}", runtime_dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(runtime_dir, std::fs::Permissions::from_mode(0o750))
            .with_context(|| {
                format!(
                    "set permissions on runtime directory {}",
                    runtime_dir.display()
                )
            })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{create_runtime_dir, InstanceLock};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn runtime_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn runtime_directory_is_not_world_writable() {
        let root = runtime_dir();
        let runtime = root.path().join("run");

        create_runtime_dir(&runtime).unwrap();

        let mode = std::fs::metadata(&runtime).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn second_instance_cannot_acquire_lock() {
        let root = runtime_dir();
        let first = InstanceLock::acquire(root.path()).unwrap();

        let error = InstanceLock::acquire(root.path()).unwrap_err();
        assert!(error
            .to_string()
            .contains("another aiqiezi-server instance"));

        drop(first);
    }

    #[test]
    fn lock_is_released_after_drop() {
        let root = runtime_dir();
        let first = InstanceLock::acquire(root.path()).unwrap();
        drop(first);

        InstanceLock::acquire(root.path()).unwrap();
    }

    #[test]
    fn stale_lock_file_does_not_block() {
        let root = runtime_dir();
        let lock_path = root.path().join("server.lock");
        std::fs::write(&lock_path, b"stale").unwrap();

        let lock = InstanceLock::acquire(root.path()).unwrap();
        let contents = std::fs::read_to_string(lock.path()).unwrap();

        assert_eq!(contents.trim(), std::process::id().to_string());
    }

    #[test]
    fn runtime_directory_is_created() {
        let root = runtime_dir();
        let runtime: &Path = &root.path().join("nested/run");

        create_runtime_dir(runtime).unwrap();
        assert!(runtime.is_dir());
    }
}
