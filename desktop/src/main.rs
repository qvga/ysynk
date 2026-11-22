#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod tray_host;

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
    settings: settings::Service,
    tray_created: AtomicBool,
    hidden_to_tray: AtomicBool,
    manual_reopen: Arc<AtomicBool>,
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

fn settings_caller(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Desktop settings are only available in the main window.".into())
    }
}

#[tauri::command]
async fn get_desktop_settings(
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
) -> Result<settings::Settings, String> {
    settings_caller(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<Desktop>();
        if state.shutting_down.load(Ordering::Acquire) {
            return Err("ysynk is shutting down.".into());
        }
        state.settings.get(tray_available(&handle))
    })
    .await
    .map_err(|_| "Could not read desktop settings. Reload before retrying.".to_owned())?
}

#[tauri::command]
async fn save_desktop_settings(
    settings: settings::Input,
    window: tauri::WebviewWindow,
    handle: tauri::AppHandle,
) -> Result<settings::Settings, String> {
    settings_caller(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<Desktop>();
        if state.shutting_down.load(Ordering::Acquire) {
            return Err("ysynk is shutting down; desktop settings were not changed.".into());
        }
        state.settings.save(settings, tray_available(&handle))
    })
    .await
    .map_err(|_| {
        "Desktop settings could not finish saving. Reload native settings before retrying."
            .to_owned()
    })?
}

fn tray_available(handle: &tauri::AppHandle) -> bool {
    handle
        .state::<Desktop>()
        .tray_created
        .load(Ordering::Acquire)
        && tray_host::available()
}

fn login_store(handle: &tauri::AppHandle, data_override: bool) -> Box<dyn settings::Autostart> {
    // The override represents a different configuration. Never inspect, remove
    // or modify the normal configuration's OS login registration from it.
    if data_override {
        return Box::new(settings::DisabledAutostart);
    }
    #[cfg(target_os = "linux")]
    {
        let executable = handle
            .env()
            .appimage
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_default();
        Box::new(settings::XdgAutostart {
            directory: dirs::config_dir(),
            executable,
        })
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        Box::new(settings::PlatformAutostart {
            handle: handle.clone(),
            executable: std::env::current_exe().unwrap_or_default(),
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Box::new(settings::DisabledAutostart)
    }
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
        handle.state::<Desktop>().settings.close();
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
    if let Some(state) = handle.try_state::<Desktop>() {
        state.manual_reopen.store(true, Ordering::Release);
        state.hidden_to_tray.store(false, Ordering::Release);
    }
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
        .tray_created
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
    let data_override = std::env::var_os("YSYNK_RESTIC_DATA_DIR").is_some();
    let login_launch = settings::is_login_launch(std::env::args_os());
    let manual_reopen = Arc::new(AtomicBool::new(false));
    let activation = manual_reopen.clone();
    let mut builder = tauri::Builder::default();
    // Register first. Overrides intentionally use only the core data-dir lock,
    // not the production app's global activation identity.
    if !data_override {
        builder = builder.plugin(tauri_plugin_single_instance::init(
            move |handle, arguments, _| {
                if !settings::is_login_launch(arguments) {
                    activation.store(true, Ordering::Release);
                    show(handle);
                }
            },
        ));
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if !data_override {
        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .app_name("ysynk")
                .build(),
        );
    }
    let application = builder
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let directory = std::env::var_os("YSYNK_RESTIC_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?.join("restic"));
            let core = App::open(directory.clone())?;
            let mut notification_cursor = None;
            // Skip completions restored from disk before admitting any new work.
            core.notification_states(&mut notification_cursor);
            let settings = settings::Service::new(Box::new(settings::FilePreferences(directory.join("desktop-settings.json"))), login_store(app.handle(), data_override));
            app.manage(Desktop { core: core.clone(), settings, tray_created: AtomicBool::new(false), hidden_to_tray: AtomicBool::new(false), manual_reopen: manual_reopen.clone(), shutting_down: AtomicBool::new(false), drained: AtomicBool::new(false), exporting_password: Arc::new(AtomicBool::new(false)) });
            // Creation is not proof that a desktop tray host is visible. The
            // user must explicitly opt in after checking the native menu.
            if setup_tray(app).is_err() {
                eprintln!("Tray unavailable; keep the window open for background scheduling.");
            }
            let available = tray_available(app.handle());
            let state = app.state::<Desktop>();
            let read_ok = state.settings.get(available).is_ok();
            if !read_ok { eprintln!("Desktop settings unavailable; starting with the window visible."); }
            if !read_ok || !settings::start_hidden(login_launch, state.settings.preferences(), available, manual_reopen.load(Ordering::Acquire)) {
                show(app.handle());
            } else {
                state.hidden_to_tray.store(true, Ordering::Release);
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                while !handle.state::<Desktop>().shutting_down.load(Ordering::Relaxed) {
                    // If a tray host disappears, a hidden app must become
                    // reachable again. Never hide later when a host appears.
                    if handle.state::<Desktop>().hidden_to_tray.load(Ordering::Acquire) && !tray_available(&handle) {
                        show(&handle);
                    }
                    core.tick();
                    for state in core.notification_states(&mut notification_cursor) {
                        notify(&state);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![request, export_repository_password, get_desktop_settings, save_desktop_settings, quit])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Desktop>();
                if state.settings.preferences().hide_to_tray && tray_available(window.app_handle()) {
                    api.prevent_close();
                    if window.hide().is_ok() {
                        state.hidden_to_tray.store(true, Ordering::Release);
                    }
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
