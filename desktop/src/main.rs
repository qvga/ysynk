#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    Manager,
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_dialog::DialogExt;
use ysynk_core::{App, Request};

struct Desktop {
    core: Arc<App>,
    hide_to_tray: AtomicBool,
    tray_available: AtomicBool,
    shutting_down: AtomicBool,
    drained: AtomicBool,
    exporting_password: Arc<AtomicBool>,
}

struct PasswordExportGuard(Arc<AtomicBool>);
impl Drop for PasswordExportGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
async fn export_repository_password(
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
    state
        .exporting_password
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "A repository password export dialog is already open.".to_owned())?;
    let guard = PasswordExportGuard(state.exporting_password.clone());
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        core.validate_repository_password_export(&repository_id, revision)
            .map_err(|error| error.to_string())?;
        let parent = handle.get_webview_window("main").ok_or_else(|| {
            "The main window is unavailable; password export was not completed.".to_owned()
        })?;
        let selected = handle
            .dialog()
            .file()
            .set_parent(&parent)
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
async fn request(
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

#[tauri::command]
fn set_hide_to_tray(enabled: bool, state: tauri::State<'_, Desktop>) -> Result<(), String> {
    if enabled && !state.tray_available.load(Ordering::Relaxed) {
        return Err(
            "A tray icon could not be created. Keep the window open for background backups.".into(),
        );
    }
    state.hide_to_tray.store(enabled, Ordering::Relaxed);
    Ok(())
}

fn begin_shutdown(handle: &tauri::AppHandle) {
    let state = handle.state::<Desktop>();
    if state.shutting_down.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    let core = state.core.clone();
    std::thread::spawn(move || {
        // The runtime retains its instance lock until its subprocess has been
        // cancelled and reaped. Never abandon a running restic process.
        core.shutdown();
        handle
            .state::<Desktop>()
            .drained
            .store(true, Ordering::SeqCst);
        handle.exit(0);
    });
}

#[tauri::command]
fn quit(handle: tauri::AppHandle) {
    begin_shutdown(&handle);
}

fn show(handle: &tauri::AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn tray_icon() -> Image<'static> {
    let mut pixels = vec![0; 24 * 24 * 4];
    for y in 0_i32..24 {
        for x in 0_i32..24 {
            let distance = (x - 11).pow(2) + (y - 11).pow(2);
            if (64..=100).contains(&distance)
                || (x == 11 && (7..=12).contains(&y))
                || (x == y - 1 && (12..=15).contains(&y))
            {
                let i = ((y * 24 + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&[140, 150, 145, 255]);
            }
        }
    }
    Image::new_owned(pixels, 24, 24)
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open ysynk", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit ysynk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::new()
        .icon(tray_icon())
        .tooltip("ysynk · encrypted backups")
        .menu(&menu)
        .on_menu_event(|handle, event| match event.id().as_ref() {
            "open" => show(handle),
            "quit" => begin_shutdown(handle),
            _ => (),
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    app.state::<Desktop>()
        .tray_available
        .store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg(target_os = "linux")]
fn notify(state: &str) {
    let Ok(connection) = dbus::blocking::Connection::new_session() else {
        return;
    };
    let body = match state {
        "success" => "The operation completed. Open ysynk to review the results.",
        "partial" => {
            "Some data or destinations were not backed up. Open ysynk to review the results."
        }
        "cancelled" => {
            "The operation was cancelled. Existing snapshots were not marked as a new successful backup."
        }
        _ => "The operation did not complete. Open ysynk to review the results.",
    };
    let _: Result<(u32,), _> = connection
        .with_proxy(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            std::time::Duration::from_secs(2),
        )
        .method_call(
            "org.freedesktop.Notifications",
            "Notify",
            (
                "ysynk",
                0_u32,
                "",
                "ysynk backup activity",
                body,
                Vec::<String>::new(),
                dbus::arg::PropMap::new(),
                -1_i32,
            ),
        );
}

#[cfg(not(target_os = "linux"))]
fn notify(_state: &str) {}

fn main() {
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let directory = std::env::var_os("YSYNK_RESTIC_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?.join("restic"));
            let core = App::open(directory)?;
            app.manage(Desktop { core: core.clone(), hide_to_tray: AtomicBool::new(false), tray_available: AtomicBool::new(false), shutting_down: AtomicBool::new(false), drained: AtomicBool::new(false), exporting_password: Arc::new(AtomicBool::new(false)) });
            // Creation is not proof that a desktop tray host is visible. The
            // user must explicitly opt in after checking the native menu.
            if setup_tray(app).is_err() {
                eprintln!("Tray unavailable; keep the window open for background scheduling.");
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut observed = std::collections::HashSet::<String>::new();
                let mut initialized = false;
                while !handle.state::<Desktop>().shutting_down.load(Ordering::Relaxed) {
                    core.tick();
                    if let Ok(status) = core.request(Request::Status) {
                        if let Some(jobs) = status["history"].as_array() {
                            let mut current = std::collections::HashSet::new();
                            for job in jobs.iter().rev() {
                                let id = job["id"].as_str().unwrap_or_default();
                                if initialized && !observed.contains(id) && matches!(job["kind"].as_str(), Some("backup" | "restore" | "check" | "apply_retention" | "prune")) {
                                    notify(job["state"].as_str().unwrap_or("failed"));
                                }
                                current.insert(id.to_owned());
                            }
                            observed = current;
                        }
                        initialized = true;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![request, export_repository_password, set_hide_to_tray, quit])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Desktop>();
                if state.hide_to_tray.load(Ordering::Relaxed) && state.tray_available.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not start ysynk; check desktop prerequisites and whether another instance is running.");
    application.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !handle.state::<Desktop>().drained.load(Ordering::SeqCst)
        {
            api.prevent_exit();
            begin_shutdown(handle);
        }
    });
}
