//! Non-secret desktop preferences and transactional login-registration changes.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub type Result<T> = std::result::Result<T, String>;
const READ_ERROR: &str = "Could not read desktop settings. Check desktop-settings.json in the app data folder, then reload settings.";
const WRITE_ERROR: &str = "Could not save desktop settings. Preferences were not changed; check app data folder access and reload settings.";
const LOGIN_ERROR: &str = "Could not read desktop-login registration. Check your operating system login settings, then reload.";

#[derive(Clone, Copy, Default, Deserialize, Serialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub start_minimized: bool,
    pub hide_to_tray: bool,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub start_at_login: bool,
    pub start_minimized: bool,
    pub hide_to_tray: bool,
}
#[derive(Debug, Serialize)]
pub struct Settings {
    pub start_at_login: bool,
    pub start_minimized: bool,
    pub hide_to_tray: bool,
    pub tray_available: bool,
    pub autostart_supported: bool,
}
pub struct Registration {
    pub enabled: bool,
    // Linux restores the exact prior entry if preference persistence fails.
    pub bytes: Option<Vec<u8>>,
}
pub trait Autostart: Send + Sync {
    fn supported(&self) -> bool;
    fn enabled(&self) -> Result<bool>;
    fn set_enabled(&self, enabled: bool) -> Result<()>;
    fn checkpoint(&self) -> Result<Registration> {
        self.enabled().map(|enabled| Registration {
            enabled,
            bytes: None,
        })
    }
    fn restore(&self, registration: &Registration) -> Result<()> {
        self.set_enabled(registration.enabled)
    }
}
pub trait PreferenceStore: Send + Sync {
    fn load(&self) -> Result<Preferences>;
    fn save(&self, preferences: Preferences) -> Result<()>;
}
pub struct FilePreferences(pub PathBuf);
impl PreferenceStore for FilePreferences {
    fn load(&self) -> Result<Preferences> {
        match bounded_read(&self.0) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).map_err(|_| READ_ERROR.into()),
            Ok(None) => Ok(Preferences::default()),
            Err(_) => Err(READ_ERROR.into()),
        }
    }
    fn save(&self, preferences: Preferences) -> Result<()> {
        let bytes = serde_json::to_vec(&preferences).map_err(|_| WRITE_ERROR.to_owned())?;
        atomic_write(&self.0, &bytes).map_err(|_| WRITE_ERROR.into())
    }
}
pub struct Service {
    preferences: Mutex<Preferences>,
    closing: std::sync::atomic::AtomicBool,
    store: Box<dyn PreferenceStore>,
    autostart: Box<dyn Autostart>,
}
impl Service {
    pub fn new(store: Box<dyn PreferenceStore>, autostart: Box<dyn Autostart>) -> Self {
        // A failed initial read never enables runtime hiding.
        Self {
            preferences: Mutex::new(Preferences::default()),
            closing: std::sync::atomic::AtomicBool::new(false),
            store,
            autostart,
        }
    }
    pub fn preferences(&self) -> Preferences {
        *self.preferences.lock().unwrap()
    }
    pub fn close(&self) {
        self.closing
            .store(true, std::sync::atomic::Ordering::Release);
        // Drain the transaction before the desktop process exits. Subsequent
        // queued settings commands fail after taking the same transaction lock.
        drop(self.preferences.lock().unwrap());
    }
    fn settings(&self, p: Preferences, enabled: bool, tray: bool) -> Settings {
        Settings {
            start_at_login: enabled,
            start_minimized: p.start_minimized,
            hide_to_tray: p.hide_to_tray,
            tray_available: tray,
            autostart_supported: self.autostart.supported(),
        }
    }
    pub fn get(&self, tray: bool) -> Result<Settings> {
        let mut runtime = self.preferences.lock().unwrap();
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return Err("ysynk is shutting down; desktop settings are unavailable.".into());
        }
        let preferences = self.store.load()?;
        let enabled = self.autostart.enabled()?;
        *runtime = preferences;
        Ok(self.settings(preferences, enabled, tray))
    }
    pub fn save(&self, input: Input, tray: bool) -> Result<Settings> {
        let mut runtime = self.preferences.lock().unwrap();
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return Err("ysynk is shutting down; desktop settings were not changed.".into());
        }
        if input.start_minimized && !input.hide_to_tray {
            return Err("Start minimized requires Hide to tray. Enable Hide to tray first.".into());
        }
        if !tray
            && ((!runtime.hide_to_tray && input.hide_to_tray)
                || (!runtime.start_minimized && input.start_minimized))
        {
            return Err("No usable tray host is available. Keep the window visible and retry after checking the desktop tray.".into());
        }
        let before = self.autostart.checkpoint()?;
        let changed = before.enabled != input.start_at_login;
        if changed && input.start_at_login && !self.autostart.supported() {
            return Err("Start at login is unavailable for this launch. Use a normal installed executable without a data-directory override or unsafe path characters.".into());
        }
        if changed && self.autostart.set_enabled(input.start_at_login).is_err() {
            return Err(self.rollback_error(
                &before,
                "Could not change desktop-login registration. Saved preferences were not changed.",
            ));
        }
        if changed && self.autostart.enabled().ok() != Some(input.start_at_login) {
            return Err(self.rollback_error(&before, "Desktop-login registration could not be verified. Saved preferences were not changed."));
        }
        let preferences = Preferences {
            start_minimized: input.start_minimized,
            hide_to_tray: input.hide_to_tray,
        };
        if self.store.save(preferences).is_err() {
            return Err(if changed {
                self.rollback_error(&before, WRITE_ERROR)
            } else {
                WRITE_ERROR.into()
            });
        }
        *runtime = preferences;
        Ok(self.settings(preferences, input.start_at_login, tray))
    }
    fn rollback_error(&self, before: &Registration, error: &str) -> String {
        if self.autostart.restore(before).is_err() {
            format!(
                "{error} Login registration could not be restored and may have changed; reload settings before retrying."
            )
        } else {
            error.into()
        }
    }
}

pub struct DisabledAutostart;
impl Autostart for DisabledAutostart {
    fn supported(&self) -> bool {
        false
    }
    fn enabled(&self) -> Result<bool> {
        Ok(false)
    }
    fn set_enabled(&self, enabled: bool) -> Result<()> {
        if enabled {
            Err("Start at login is disabled for data-directory overrides.".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
pub struct XdgAutostart {
    pub directory: Option<PathBuf>,
    pub executable: PathBuf,
}
#[cfg(target_os = "linux")]
impl XdgAutostart {
    fn path(&self) -> Result<PathBuf> {
        self.directory
            .as_ref()
            .filter(|path| path.is_absolute())
            .map(|path| path.join("autostart/ysynk.desktop"))
            .ok_or_else(|| "Could not locate the desktop-login configuration folder.".into())
    }
    fn inspect(&self, bytes: &[u8]) -> Result<bool> {
        use std::collections::HashMap;
        let text = std::str::from_utf8(bytes).map_err(|_| LOGIN_ERROR.to_owned())?;
        let mut fields = HashMap::new();
        let mut entry = false;
        let mut found_entry = false;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            if line.starts_with('[') {
                entry = line == "[Desktop Entry]";
                if entry && found_entry {
                    return Err(LOGIN_ERROR.into());
                }
                found_entry |= entry;
            } else if entry {
                let (key, value) = line.split_once('=').ok_or_else(|| LOGIN_ERROR.to_owned())?;
                if fields.insert(key, value).is_some() {
                    return Err(LOGIN_ERROR.into());
                }
            }
        }
        if fields.get("Type") != Some(&"Application") || fields.get("Name") != Some(&"ysynk") {
            return Err(LOGIN_ERROR.into());
        }
        let executable = fields
            .get("Exec")
            .and_then(|exec| exec.strip_suffix(" --autostart"))
            .ok_or_else(|| LOGIN_ERROR.to_owned())?;
        if !safe_executable(Path::new(executable))
            || [
                "OnlyShowIn",
                "NotShowIn",
                "TryExec",
                "AutostartCondition",
                "X-KDE-autostart-condition",
            ]
            .iter()
            .any(|key| fields.contains_key(key))
        {
            return Err("The ysynk login entry was changed outside the app and cannot be verified. Inspect it in your desktop login settings before retrying.".into());
        }
        if Path::new(executable) != self.executable {
            return Err("The login entry points to a different executable. Move ysynk.desktop out of your autostart folder, then reload settings and enable Start at login for this installation.".into());
        }
        for key in ["Hidden", "X-GNOME-Autostart-enabled"] {
            if fields
                .get(key)
                .is_some_and(|value| !["true", "false"].contains(value))
            {
                return Err(LOGIN_ERROR.into());
            }
        }
        Ok(fields.get("Hidden") != Some(&"true")
            && fields.get("X-GNOME-Autostart-enabled") != Some(&"false"))
    }
}
#[cfg(target_os = "linux")]
impl Autostart for XdgAutostart {
    fn supported(&self) -> bool {
        self.path().is_ok() && safe_executable(&self.executable)
    }
    fn enabled(&self) -> Result<bool> {
        self.checkpoint().map(|registration| registration.enabled)
    }
    fn checkpoint(&self) -> Result<Registration> {
        let bytes = bounded_read(&self.path()?).map_err(|_| LOGIN_ERROR.to_owned())?;
        let enabled = bytes
            .as_deref()
            .map(|bytes| self.inspect(bytes))
            .transpose()?
            .unwrap_or(false);
        Ok(Registration { enabled, bytes })
    }
    fn set_enabled(&self, enabled: bool) -> Result<()> {
        let path = self.path()?;
        // Reject unsafe Exec syntax before creating a folder or opening a file.
        if enabled && !safe_executable(&self.executable) {
            return Err("Start at login requires an executable path without spaces, quotes, percent signs, backslashes or other desktop-entry control characters.".into());
        }
        // Refuse to replace another entry, a symlink, or unreadable contents.
        let _ = self.checkpoint()?;
        if enabled {
            fs::create_dir_all(path.parent().unwrap())
                .map_err(|_| "Could not create the desktop-login folder.".to_owned())?;
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName=ysynk\nExec={} --autostart\nTerminal=false\n",
                self.executable.to_str().unwrap()
            );
            atomic_write(&path, entry.as_bytes())
                .map_err(|_| "Could not register ysynk for desktop login.".into())
        } else {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err("Could not remove ysynk desktop-login registration.".into()),
            }
        }
    }
    fn restore(&self, registration: &Registration) -> Result<()> {
        let path = self.path()?;
        // The entry may have changed externally while the preferences were saved.
        // Apply the same identity check used before a normal registration change.
        let _ = self.checkpoint()?;
        if let Some(bytes) = &registration.bytes {
            atomic_write(&path, bytes)
                .map_err(|_| "Could not restore desktop-login registration.".into())
        } else {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err("Could not restore desktop-login registration.".into()),
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct PlatformAutostart {
    pub handle: tauri::AppHandle,
    pub executable: PathBuf,
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Autostart for PlatformAutostart {
    fn supported(&self) -> bool {
        safe_executable(&self.executable)
    }
    fn enabled(&self) -> Result<bool> {
        use tauri_plugin_autostart::ManagerExt;
        self.handle
            .autolaunch()
            .is_enabled()
            .map_err(|_| LOGIN_ERROR.into())
    }
    fn set_enabled(&self, enabled: bool) -> Result<()> {
        use tauri_plugin_autostart::ManagerExt;
        if enabled && !self.supported() {
            return Err(
                "The executable path cannot be represented safely in login registration.".into(),
            );
        }
        if enabled {
            self.handle.autolaunch().enable()
        } else {
            self.handle.autolaunch().disable()
        }
        .map_err(|_| "Could not change desktop-login registration.".into())
    }
}

/// Also guards the unescaped platform-plugin formats on Windows and macOS.
/// Unsupported paths remain usable for manual launches, not login registration.
pub fn safe_executable(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|text| {
            text.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || b"/._-+".contains(&b)
                    || (cfg!(windows) && b"\\:".contains(&b))
            })
        })
}
pub fn start_hidden(
    login: bool,
    preferences: Preferences,
    tray: bool,
    manual_reopen: bool,
) -> bool {
    login && preferences.start_minimized && preferences.hide_to_tray && tray && !manual_reopen
}
pub fn is_login_launch(arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> bool {
    arguments
        .into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == "--autostart")
}

pub fn bounded_read(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("settings must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(std::io::Error::other("settings exceed size limit"));
    }
    Ok(Some(bytes))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    // Commit has happened. A directory-flush failure must not report that the
    // preferences were unchanged; file contents were synced before publication.
    #[cfg(unix)]
    if let Ok(parent) = fs::File::open(parent) {
        let _ = parent.sync_all();
    }
    Ok(())
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
