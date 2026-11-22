use crate::{Desktop, settings};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use ysynk_core::Request;

struct FileDialogGuard(Arc<AtomicBool>);
impl FileDialogGuard {
    fn acquire(state: &Desktop) -> Result<Self, String> {
        state
            .file_dialog_open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Another native file dialog is already open.".to_owned())?;
        Ok(Self(state.file_dialog_open.clone()))
    }
}
impl Drop for FileDialogGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub(super) async fn export_repository_password(
    repository_id: String,
    revision: u64,
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
    state: tauri::State<'_, Desktop>,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Repository passwords can only be exported from the main window.".into());
    }
    if state.shutting_down.load(Ordering::Acquire) {
        return Err("ysynk is shutting down; password export was not completed.".into());
    }
    let guard = FileDialogGuard::acquire(&state)?;
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        core.validate_repository_password_export(&repository_id, revision)
            .map_err(|error| error.to_string())?;
        let selected = handle
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Export repository password to a private recovery file")
            .set_file_name("ysynk-repository-password.txt")
            .blocking_save_file();
        // Cancellation never accesses the native credential store. The picker
        // does not reserve core state: regular work can continue while open.
        let Some(selected) = selected else {
            return Ok(false);
        };
        if handle
            .state::<Desktop>()
            .shutting_down
            .load(Ordering::Acquire)
        {
            return Err("ysynk is shutting down; password export was not completed.".into());
        }
        let path = selected
            .into_path()
            .map_err(|_| "Select a new password file in a local private folder.".to_owned())?;
        core.export_repository_password(&repository_id, revision, path)
            .map_err(|error| error.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|_| {
        "Password export could not finish; inspect the selected folder before retrying.".to_owned()
    })?
}

#[tauri::command]
pub(super) async fn select_restic_executable(
    revision: u64,
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
    state: tauri::State<'_, Desktop>,
) -> Result<Option<serde_json::Value>, String> {
    require_main_window(&window)?;
    if state.shutting_down.load(Ordering::Acquire) {
        return Err("ysynk is shutting down.".into());
    }
    let guard = FileDialogGuard::acquire(&state)?;
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let Some(selected) = handle
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(
                "Choose the trusted restic executable that will receive repository passwords",
            )
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| "Select a local executable.".to_owned())?;
        if handle
            .state::<Desktop>()
            .shutting_down
            .load(Ordering::Acquire)
        {
            return Err("ysynk is shutting down.".into());
        }
        core.set_restic_executable(revision, &path)
            .map(Some)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Could not finish selecting the restic executable.".to_owned())?
}

#[tauri::command]
pub(super) async fn request(
    request: Request,
    state: tauri::State<'_, Desktop>,
) -> Result<serde_json::Value, String> {
    if state.shutting_down.load(Ordering::Relaxed) {
        return Err("ysynk is shutting down. Wait for the current operation to stop.".into());
    }
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || core.request(request))
        .await
        .map_err(|_| {
            "The operation could not finish. Reopen ysynk and inspect its history.".to_owned()
        })?
        .map_err(|error| error.to_string())
}

fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This operation is only available in the main window.".into())
    }
}

#[tauri::command]
pub(super) async fn get_desktop_settings(
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
) -> Result<settings::Settings, String> {
    require_main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<Desktop>();
        if state.shutting_down.load(Ordering::Acquire) {
            return Err("ysynk is shutting down.".into());
        }
        state.settings.get(crate::tray::available(&handle))
    })
    .await
    .map_err(|_| "Could not read desktop settings. Reload before retrying.".to_owned())?
}

#[tauri::command]
pub(super) async fn save_desktop_settings(
    settings: settings::Input,
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
) -> Result<settings::Settings, String> {
    require_main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<Desktop>();
        if state.shutting_down.load(Ordering::Acquire) {
            return Err("ysynk is shutting down; desktop settings were not changed.".into());
        }
        state
            .settings
            .save(settings, crate::tray::available(&handle))
    })
    .await
    .map_err(|_| {
        "Desktop settings could not finish saving. Reload native settings before retrying."
            .to_owned()
    })?
}

#[tauri::command]
pub(super) fn quit(handle: tauri::AppHandle) {
    crate::begin_shutdown(&handle);
}
