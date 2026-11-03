//! Persistent native credentials only. No deletion, migration, fallback, or session cache.
use crate::Repository;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "io.ysynk.restic";
pub(crate) const LOCKED: &str =
    "System keychain is locked; unlock it in your desktop keychain manager, then retry";
pub(crate) const UNAVAILABLE: &str = "System keychain is unavailable; sign in to a desktop session with a persistent native keychain and retry";
const WRITE_FAILED: &str = "Could not save the repository credential in the system keychain; check keychain access and retry initialization";

pub(crate) fn creation_lock(directory: &std::path::Path, account: &str) -> Result<std::fs::File> {
    use std::fs::{self, OpenOptions};
    ensure!(crate::config::valid_id(account), "invalid repository ID");
    fs::create_dir_all(directory)?;
    ensure!(
        fs::symlink_metadata(directory)?.is_dir(),
        "credential lock directory must not be a symlink"
    );
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        ensure!(
            fs::metadata(directory)?.uid() == unsafe { libc::geteuid() },
            "credential lock directory must be owned by this user"
        );
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(directory.join(format!("{account}.lock")))?;
    ensure!(
        file.metadata()?.is_file(),
        "credential lock must be a regular file"
    );
    fs2::FileExt::try_lock_exclusive(&file)
        .context("Repository initialization is already running in another app instance; retry after it completes")?;
    // Never unlink: waiters must continue to lock the same inode.
    Ok(file)
}

pub(crate) trait Store: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn create(&self, account: &str, secret: &[u8]) -> Result<()>;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    version: u32,
    location: String,
    pub(crate) password: String,
}
impl Drop for Record {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}
impl Record {
    pub(crate) fn generate(repo: &Repository) -> Self {
        // Two OS-CSPRNG UUIDs provide 244 random bits, without user input.
        Self {
            version: 1,
            location: repo.location.clone(),
            password: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        }
    }
    pub(crate) fn bytes(&self) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(serde_json::to_vec(self)?))
    }
}
pub(crate) fn read(store: &dyn Store, repo: &Repository) -> Result<Option<Record>> {
    let Some(bytes) = store.get(&repo.id)? else {
        return Ok(None);
    };
    let record: Record = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid repository credential in system keychain; restore the native keychain record before retrying"))?;
    ensure!(
        record.version == 1 && record.location == repo.location,
        "System keychain credential belongs to another location or version; use a new repository ID for a different location"
    );
    ensure!(
        record.password.len() == 64 && record.password.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid repository password record in system keychain"
    );
    Ok(Some(record))
}
pub(crate) fn required(store: &dyn Store, repo: &Repository) -> Result<Record> {
    read(store, repo)?.ok_or_else(|| anyhow::anyhow!("Repository credential is missing from the system keychain; initialize a new repository or restore its native keychain record"))
}

pub(crate) struct NativeStore;

#[cfg(target_os = "linux")]
impl Store for NativeStore {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        use dbus_secret_service::{EncryptionType, SecretService};
        use std::collections::HashMap;
        // Unlike keyring's convenience backend, never unlock an item. In 4.1.0,
        // timeout zero also returns BEFORE sending Prompt, including create races.
        // Plain sessions trust the local user session bus. The dependency's DH
        // implementation uses non-cryptographic randomness; do not enable it.
        let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Plain, 0)
            .map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
        // Search only the persistent default collection, never service-wide
        // results that could include transient session-collection entries.
        let collection = persistent_collection(&service)?;
        let items = collection
            .search_items(HashMap::from([("service", SERVICE), ("username", account)]))
            .map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
        ensure!(
            items.len() <= 1,
            "Duplicate system keychain entries; resolve them in your native keychain manager before retrying"
        );
        match items.first() {
            Some(item) => {
                ensure!(
                    !item.is_locked().map_err(|_| anyhow::anyhow!(UNAVAILABLE))?,
                    LOCKED
                );
                item.get_secret()
                    .map(|bytes| Some(Zeroizing::new(bytes)))
                    .map_err(native_error)
            }
            None => {
                // Do not interpret an empty search during a lock race as
                // permission to generate a replacement credential.
                ensure!(!collection.is_locked().map_err(native_error)?, LOCKED);
                Ok(None)
            }
        }
    }
    fn create(&self, account: &str, secret: &[u8]) -> Result<()> {
        use dbus_secret_service::{EncryptionType, SecretService};
        use std::collections::HashMap;
        ensure!(
            self.get(account)?.is_none(),
            "Repository credential already exists; retry initialization"
        );
        let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Plain, 0)
            .map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
        // Never use a session collection or create/unlock a collection.
        let collection = persistent_collection(&service)?;
        collection
            .create_item(
                "Ysynk restic repository",
                HashMap::from([("service", SERVICE), ("username", account)]),
                secret,
                false,
                "application/json",
            )
            .map_err(|_| anyhow::anyhow!(WRITE_FAILED))?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn persistent_collection(
    service: &dbus_secret_service::SecretService,
) -> Result<dbus_secret_service::Collection<'_>> {
    use dbus_secret_service::Error;
    let collection = service.get_default_collection().map_err(native_error)?;
    // The default alias must not have been configured to point to the reserved
    // transient session collection. No fallback is permitted if either query
    // fails; a missing session alias is normal.
    match service.get_collection_by_alias("session") {
        Ok(session) => ensure!(
            collection.path != session.path,
            "System keychain default collection is temporary; select a persistent default collection in your native keychain manager"
        ),
        Err(Error::NoResult) => {}
        Err(error) => return Err(native_error(error)),
    }
    ensure!(!collection.is_locked().map_err(native_error)?, LOCKED);
    Ok(collection)
}

#[cfg(target_os = "linux")]
fn native_error(error: dbus_secret_service::Error) -> anyhow::Error {
    use dbus_secret_service::Error;
    match error {
        Error::Locked | Error::Prompt => anyhow::anyhow!(LOCKED),
        Error::Dbus(ref error) if error.name() == Some("org.freedesktop.Secret.Error.IsLocked") => {
            anyhow::anyhow!(LOCKED)
        }
        _ => anyhow::anyhow!(UNAVAILABLE),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn entry(account: &str) -> Result<keyring::Entry> {
    // Explicit constructors forbid keyring's unsupported-platform mock default.
    #[cfg(target_os = "macos")]
    let credential = keyring::macos::MacCredential::new_with_target(None, SERVICE, account);
    #[cfg(target_os = "windows")]
    let credential = keyring::windows::WinCredential::new_with_target(None, SERVICE, account);
    credential
        .map(|value| keyring::Entry::new_with_credential(Box::new(value)))
        .map_err(|_| anyhow::anyhow!(UNAVAILABLE))
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn without_prompt<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::os::macos::keychain::SecKeychain;
        // Keychain Services' interaction switch is process-wide. Serialize our
        // accesses and restore its prior setting after the native operation.
        static ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _access = ACCESS.lock().map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
        let allowed =
            SecKeychain::user_interaction_allowed().map_err(|_| anyhow::anyhow!(UNAVAILABLE))?;
        let _interaction = if allowed {
            Some(
                SecKeychain::disable_user_interaction()
                    .map_err(|_| anyhow::anyhow!(UNAVAILABLE))?,
            )
        } else {
            None
        };
        operation()
    }
    #[cfg(target_os = "windows")]
    operation()
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Store for NativeStore {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        without_prompt(|| match entry(account)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(_)) => anyhow::bail!(LOCKED),
            Err(_) => anyhow::bail!(UNAVAILABLE),
        })
    }
    fn create(&self, account: &str, secret: &[u8]) -> Result<()> {
        ensure!(
            self.get(account)?.is_none(),
            "Repository credential already exists; retry initialization"
        );
        without_prompt(|| {
            entry(account)?
                .set_secret(secret)
                .map_err(|_| anyhow::anyhow!(WRITE_FAILED))
        })
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Store for NativeStore {
    fn get(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        anyhow::bail!(UNAVAILABLE)
    }
    fn create(&self, _: &str, _: &[u8]) -> Result<()> {
        anyhow::bail!(UNAVAILABLE)
    }
}
