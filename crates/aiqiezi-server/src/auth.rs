/*
  crates/aiqiezi-server/src/auth.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use iroh::EndpointId;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const AUTH_FILE: &str = "auth.json";
const PASSWORD_MIN_LEN: usize = 8;
pub(crate) const PASSWORD_MAX_LEN: usize = 128;
pub(crate) const MAX_LOGIN_FAILURES: u32 = 5;
#[cfg(test)]
pub(crate) const BOOTSTRAP_TEST_PASSWORD: &str = "ChangeMe-123!";
const LOGIN_LOCKOUT: Duration = Duration::from_secs(30);
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const LOGIN_FAILURE_MAP_LIMIT: usize = 1024;

#[derive(Clone, Serialize, Deserialize)]
struct AuthFile {
    password_hash: String,
    must_change_password: bool,
    #[serde(default)]
    clients: Vec<String>,
}

#[derive(Clone)]
struct AuthData {
    password_hash: String,
    must_change_password: bool,
    clients: HashSet<EndpointId>,
    revision: u64,
}

impl fmt::Debug for AuthData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthData")
            .field("password_hash", &"<redacted>")
            .field("must_change_password", &self.must_change_password)
            .field("clients", &self.clients.len())
            .finish()
    }
}

impl AuthData {
    fn from_file(file: AuthFile) -> anyhow::Result<Self> {
        if PasswordHash::new(&file.password_hash).is_err() {
            bail!("auth file contains an invalid password hash");
        }

        let mut clients = HashSet::new();
        for value in file.clients {
            match value.parse::<EndpointId>() {
                Ok(endpoint_id) => {
                    clients.insert(endpoint_id);
                }
                Err(error) => {
                    warn!("ignoring invalid client EndpointId in auth file: {error}");
                }
            }
        }

        Ok(Self {
            password_hash: file.password_hash,
            must_change_password: file.must_change_password,
            clients,
            revision: 0,
        })
    }

    fn to_file(&self) -> AuthFile {
        let mut clients: Vec<String> = self.clients.iter().map(ToString::to_string).collect();
        clients.sort();
        AuthFile {
            password_hash: self.password_hash.clone(),
            must_change_password: self.must_change_password,
            clients,
        }
    }
}

#[derive(Debug)]
struct LoginFailure {
    count: u32,
    last_failure: Instant,
}

#[derive(Debug, Clone)]
pub struct AuthState {
    data: Arc<RwLock<AuthData>>,
    keys_dir: PathBuf,
    login_failures: Arc<Mutex<HashMap<EndpointId, LoginFailure>>>,
    verification_permits: Arc<tokio::sync::Semaphore>,
}

impl AuthState {
    pub fn load_or_initialize(
        keys_dir: &Path,
        bootstrap_password: Option<&str>,
    ) -> anyhow::Result<Self> {
        create_private_dir(keys_dir)?;
        let auth_path = auth_file_path(keys_dir);

        let data = if auth_path.exists() {
            ensure_private_file(&auth_path)?;
            if bootstrap_password.is_some() {
                warn!(
                    "AIQIEZI_BOOTSTRAP_PASSWORD is set but {} exists; ignoring the bootstrap password",
                    auth_path.display()
                );
            }
            let file = read_auth_file(&auth_path)?;
            AuthData::from_file(file)?
        } else {
            let password = bootstrap_password.ok_or_else(|| {
                anyhow::anyhow!(
                    "no auth state found at {}; set AIQIEZI_BOOTSTRAP_PASSWORD or run `aiqiezi-server set-password`",
                    auth_path.display()
                )
            })?;
            validate_password_strength(password).map_err(anyhow::Error::msg)?;
            let password_hash = hash_password(password).map_err(anyhow::Error::msg)?;
            let data = AuthData {
                password_hash,
                must_change_password: true,
                clients: HashSet::new(),
                revision: 0,
            };
            save_auth_file(keys_dir, &data)?;
            info!("bootstrap password hash created; password change required");
            data
        };

        info!(
            clients = data.clients.len(),
            password_change_required = data.must_change_password,
            "auth state loaded"
        );

        Ok(Self {
            data: Arc::new(RwLock::new(data)),
            keys_dir: keys_dir.to_path_buf(),
            login_failures: Arc::new(Mutex::new(HashMap::new())),
            verification_permits: Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }

    pub fn must_change_password(&self) -> bool {
        self.data
            .read()
            .expect("auth state lock poisoned")
            .must_change_password
    }

    pub fn client_count(&self) -> usize {
        self.data
            .read()
            .expect("auth state lock poisoned")
            .clients
            .len()
    }

    pub fn is_client_allowed(&self, endpoint_id: &EndpointId) -> bool {
        self.data
            .read()
            .expect("auth state lock poisoned")
            .clients
            .contains(endpoint_id)
    }

    pub fn password_hash(&self) -> String {
        self.data
            .read()
            .expect("auth state lock poisoned")
            .password_hash
            .clone()
    }

    pub fn change_password(&self, new_hash: String, caller: EndpointId) -> anyhow::Result<()> {
        let mut data = self.data.write().expect("auth state lock poisoned");
        let next = AuthData {
            password_hash: new_hash,
            must_change_password: false,
            clients: HashSet::from([caller]),
            revision: data.revision.saturating_add(1),
        };
        save_auth_file(&self.keys_dir, &next)?;
        *data = next;
        Ok(())
    }

    pub fn reset_password(&self, new_hash: String, keep_clients: bool) -> anyhow::Result<()> {
        let mut data = self.data.write().expect("auth state lock poisoned");
        let mut next = data.clone();
        next.password_hash = new_hash;
        next.must_change_password = false;
        next.revision = data.revision.saturating_add(1);
        if !keep_clients {
            next.clients.clear();
        }
        save_auth_file(&self.keys_dir, &next)?;
        *data = next;
        Ok(())
    }

    pub fn password_revision(&self) -> u64 {
        self.data.read().expect("auth state lock poisoned").revision
    }

    pub fn enroll_client_if_revision(
        &self,
        endpoint_id: EndpointId,
        revision: u64,
    ) -> anyhow::Result<bool> {
        let mut data = self.data.write().expect("auth state lock poisoned");
        if data.revision != revision {
            return Ok(false);
        }
        if data.clients.contains(&endpoint_id) {
            return Ok(true);
        }

        let mut next = data.clone();
        next.clients.insert(endpoint_id);
        save_auth_file(&self.keys_dir, &next)?;
        *data = next;
        Ok(true)
    }

    pub fn login_retry_after(&self, endpoint_id: &EndpointId) -> Option<u64> {
        let failures = self
            .login_failures
            .lock()
            .expect("login failure lock poisoned");
        let failure = failures.get(endpoint_id)?;
        if failure.count < MAX_LOGIN_FAILURES {
            return None;
        }

        let elapsed = failure.last_failure.elapsed();
        if elapsed >= LOGIN_LOCKOUT {
            return None;
        }

        Some((LOGIN_LOCKOUT - elapsed).as_secs().max(1))
    }

    pub fn record_login_failure(&self, endpoint_id: EndpointId) {
        let mut failures = self
            .login_failures
            .lock()
            .expect("login failure lock poisoned");

        if failures.len() >= LOGIN_FAILURE_MAP_LIMIT {
            failures.retain(|_, failure| failure.last_failure.elapsed() < LOGIN_FAILURE_WINDOW);
        }

        let failure = failures.entry(endpoint_id).or_insert(LoginFailure {
            count: 0,
            last_failure: Instant::now(),
        });
        if failure.last_failure.elapsed() > LOGIN_FAILURE_WINDOW {
            failure.count = 0;
        }
        failure.count = failure.count.saturating_add(1);
        failure.last_failure = Instant::now();
    }

    pub fn clear_login_failures(&self, endpoint_id: &EndpointId) {
        self.login_failures
            .lock()
            .expect("login failure lock poisoned")
            .remove(endpoint_id);
    }

    pub fn clear_all_login_failures(&self) {
        self.login_failures
            .lock()
            .expect("login failure lock poisoned")
            .clear();
    }

    pub async fn verify_password(&self, password: String) -> Result<bool, String> {
        let _permit = self
            .verification_permits
            .acquire()
            .await
            .map_err(|_| "password verifier is unavailable".to_string())?;
        let password_hash = self.password_hash();
        tokio::task::spawn_blocking(move || verify_password_hash(&password_hash, &password))
            .await
            .map_err(|error| format!("password verification task failed: {error}"))
    }

    pub async fn hash_password(&self, password: String) -> Result<String, String> {
        let _permit = self
            .verification_permits
            .acquire()
            .await
            .map_err(|_| "password hasher is unavailable".to_string())?;
        tokio::task::spawn_blocking(move || hash_password(&password))
            .await
            .map_err(|error| format!("password hashing task failed: {error}"))?
    }
}

pub fn auth_file_path(keys_dir: &Path) -> PathBuf {
    keys_dir.join(AUTH_FILE)
}

pub fn password_hash_exists(keys_dir: &Path) -> bool {
    auth_file_path(keys_dir).exists()
}

pub fn set_password_offline(
    keys_dir: &Path,
    old_password: Option<&str>,
    new_password: &str,
    keep_clients: bool,
) -> anyhow::Result<()> {
    create_private_dir(keys_dir)?;
    let auth_path = auth_file_path(keys_dir);

    let mut clients = HashSet::new();
    if auth_path.exists() {
        ensure_private_file(&auth_path)?;
        let current = AuthData::from_file(read_auth_file(&auth_path)?)?;
        let old_password =
            old_password.ok_or_else(|| anyhow::anyhow!("current password is required"))?;
        if !verify_password_hash(&current.password_hash, old_password) {
            bail!("current password is invalid");
        }
        if verify_password_hash(&current.password_hash, new_password) {
            bail!("new password must differ from the current password");
        }
        if keep_clients {
            clients = current.clients;
        }
    }

    validate_password_strength(new_password).map_err(anyhow::Error::msg)?;
    let password_hash = hash_password(new_password).map_err(anyhow::Error::msg)?;
    let next = AuthData {
        password_hash,
        must_change_password: false,
        clients,
        revision: 0,
    };
    save_auth_file(keys_dir, &next)?;
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("hash password: {error}"))
}

pub fn verify_password_hash(password_hash: &str, password: &str) -> bool {
    let parsed = match PasswordHash::new(password_hash) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub fn validate_password_strength(password: &str) -> Result<(), String> {
    let length = password.chars().count();
    if !(PASSWORD_MIN_LEN..=PASSWORD_MAX_LEN).contains(&length) {
        return Err(format!(
            "password must be {PASSWORD_MIN_LEN}-{PASSWORD_MAX_LEN} characters"
        ));
    }
    if password.chars().any(char::is_control) {
        return Err("password must not contain control characters".to_string());
    }
    if !password.chars().any(|character| character.is_uppercase()) {
        return Err("password must contain at least one uppercase letter".to_string());
    }
    if !password.chars().any(|character| character.is_lowercase()) {
        return Err("password must contain at least one lowercase letter".to_string());
    }
    if !password.chars().any(|character| character.is_ascii_digit()) {
        return Err("password must contain at least one digit".to_string());
    }
    if !password
        .chars()
        .any(|character| !character.is_alphanumeric())
    {
        return Err("password must contain at least one special character".to_string());
    }
    Ok(())
}

fn read_auth_file(path: &Path) -> anyhow::Result<AuthFile> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("read auth state {}", path.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("parse auth state {}", path.display()))
}

fn save_auth_file(keys_dir: &Path, data: &AuthData) -> anyhow::Result<()> {
    create_private_dir(keys_dir)?;
    let path = auth_file_path(keys_dir);
    let encoded = serde_json::to_vec_pretty(&data.to_file()).context("encode auth state")?;
    write_private_file(&path, &encoded)
}

fn create_private_dir(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("create auth directory {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}

fn ensure_private_file(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)
        .with_context(|| format!("write temporary file {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set permissions on {}", temporary.display()))?;
    }
    fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        auth_file_path, hash_password, set_password_offline, validate_password_strength,
        verify_password_hash, AuthState, LOGIN_LOCKOUT, MAX_LOGIN_FAILURES,
    };
    use iroh::SecretKey;

    const BOOTSTRAP: &str = "ChangeMe-123!";
    const NEW_PASSWORD: &str = "NewPass-456!";

    fn endpoint_id() -> iroh::EndpointId {
        SecretKey::generate().public()
    }

    #[test]
    fn missing_bootstrap_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(AuthState::load_or_initialize(dir.path(), None).is_err());
    }

    #[test]
    fn weak_bootstrap_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(AuthState::load_or_initialize(dir.path(), Some("short")).is_err());
    }

    #[test]
    fn bootstrap_creates_hash_and_requires_change() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();

        assert!(state.must_change_password());
        assert_eq!(state.client_count(), 0);
        assert!(auth_file_path(dir.path()).exists());
        assert!(verify_password_hash(&state.password_hash(), BOOTSTRAP));
    }

    #[test]
    fn existing_auth_file_ignores_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some("OtherPass-789!")).unwrap();

        assert!(verify_password_hash(&state.password_hash(), BOOTSTRAP));
        assert!(!verify_password_hash(
            &state.password_hash(),
            "OtherPass-789!"
        ));
    }

    #[test]
    fn change_password_resets_and_enrolls_caller() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();

        let new_hash = hash_password(NEW_PASSWORD).unwrap();
        state.change_password(new_hash, caller).unwrap();

        assert!(!state.must_change_password());
        assert!(state.is_client_allowed(&caller));
        assert_eq!(state.client_count(), 1);
        assert!(verify_password_hash(&state.password_hash(), NEW_PASSWORD));
        assert!(!verify_password_hash(&state.password_hash(), BOOTSTRAP));
    }

    #[test]
    fn reset_password_respects_keep_clients() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();
        state
            .enroll_client_if_revision(caller, state.password_revision())
            .unwrap();

        state
            .reset_password(hash_password(NEW_PASSWORD).unwrap(), true)
            .unwrap();
        assert!(state.is_client_allowed(&caller));
        assert_eq!(state.client_count(), 1);
        assert!(!state.must_change_password());
        assert!(verify_password_hash(&state.password_hash(), NEW_PASSWORD));

        state
            .reset_password(hash_password("NewerPass-789!").unwrap(), false)
            .unwrap();
        assert_eq!(state.client_count(), 0);
        assert!(!state.is_client_allowed(&caller));
        assert!(verify_password_hash(
            &state.password_hash(),
            "NewerPass-789!"
        ));
    }

    #[test]
    fn enrollment_rejects_a_password_revision_change() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();
        let revision = state.password_revision();

        state
            .reset_password(hash_password(NEW_PASSWORD).unwrap(), true)
            .unwrap();
        assert_ne!(state.password_revision(), revision);
        assert!(!state.enroll_client_if_revision(caller, revision).unwrap());
        assert_eq!(state.client_count(), 0);
        assert!(state
            .enroll_client_if_revision(caller, state.password_revision())
            .unwrap());
        assert_eq!(state.client_count(), 1);
    }

    #[test]
    fn whitelist_persists_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();
        state
            .enroll_client_if_revision(caller, state.password_revision())
            .unwrap();
        drop(state);

        let state = AuthState::load_or_initialize(dir.path(), None).unwrap();
        assert!(state.is_client_allowed(&caller));
        assert_eq!(state.client_count(), 1);
    }

    #[test]
    fn offline_password_change_clears_clients() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();
        state
            .enroll_client_if_revision(caller, state.password_revision())
            .unwrap();
        drop(state);

        set_password_offline(dir.path(), Some(BOOTSTRAP), NEW_PASSWORD, false).unwrap();

        let state = AuthState::load_or_initialize(dir.path(), None).unwrap();
        assert_eq!(state.client_count(), 0);
        assert!(verify_password_hash(&state.password_hash(), NEW_PASSWORD));
    }

    #[test]
    fn offline_password_change_can_keep_clients() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();
        state
            .enroll_client_if_revision(caller, state.password_revision())
            .unwrap();
        drop(state);

        set_password_offline(dir.path(), Some(BOOTSTRAP), NEW_PASSWORD, true).unwrap();

        let state = AuthState::load_or_initialize(dir.path(), None).unwrap();
        assert_eq!(state.client_count(), 1);
        assert!(state.is_client_allowed(&caller));
        assert!(verify_password_hash(&state.password_hash(), NEW_PASSWORD));
    }

    #[test]
    fn offline_password_change_requires_current_password() {
        let dir = tempfile::tempdir().unwrap();
        AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        assert!(set_password_offline(dir.path(), None, NEW_PASSWORD, false).is_err());
        assert!(set_password_offline(dir.path(), Some("wrong"), NEW_PASSWORD, false).is_err());
    }

    #[test]
    fn login_rate_limit_blocks_after_failures() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let caller = endpoint_id();

        for _ in 0..MAX_LOGIN_FAILURES {
            state.record_login_failure(caller);
        }
        let retry = state.login_retry_after(&caller).unwrap();
        assert!(retry > 0);
        assert!(retry <= LOGIN_LOCKOUT.as_secs());

        state.clear_login_failures(&caller);
        assert!(state.login_retry_after(&caller).is_none());
    }

    #[test]
    fn password_strength_rules_are_enforced() {
        assert!(validate_password_strength(BOOTSTRAP).is_ok());
        assert!(validate_password_strength("short").is_err());
        assert!(validate_password_strength("lowercaseonly1!").is_err());
        assert!(validate_password_strength("UPPERCASEONLY1!").is_err());
        assert!(validate_password_strength("NoDigitsHere!").is_err());
        assert!(validate_password_strength("NoSpecial123").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_permissions_are_restrictive() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        AuthState::load_or_initialize(&keys_dir, Some(BOOTSTRAP)).unwrap();

        let dir_mode = std::fs::metadata(&keys_dir).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(auth_file_path(&keys_dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn debug_output_redacts_password_hash() {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::load_or_initialize(dir.path(), Some(BOOTSTRAP)).unwrap();
        let debug = format!("{state:?}");
        assert!(!debug.contains(&state.password_hash()));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn hash_verification_rejects_bad_hash() {
        assert!(!verify_password_hash("not-a-hash", BOOTSTRAP));
        assert!(!verify_password_hash(
            &hash_password(BOOTSTRAP).unwrap(),
            "wrong"
        ));
    }
}
