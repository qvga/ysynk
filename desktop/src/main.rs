#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod settings;
mod tray;
mod tray_host;

use std::{
    ffi::OsString,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::Manager;
use ysynk_core::App;

struct Desktop {
    core: Arc<App>,
    settings: settings::Service,
    tray_created: AtomicBool,
    hidden_to_tray: AtomicBool,
    shutting_down: AtomicBool,
    drained: AtomicBool,
    file_dialog_open: Arc<AtomicBool>,
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

fn show(handle: &tauri::AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        if window.show().is_ok() {
            if let Some(state) = handle.try_state::<Desktop>() {
                state.hidden_to_tray.store(false, Ordering::Release);
            }
            let _ = window.set_focus();
        }
    }
}

fn setup(
    app: &mut tauri::App,
    directory_override: Option<OsString>,
    login_launch: bool,
    manual_reopen: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let autostart = login_store(app.handle(), directory_override.is_some());
    let directory = match directory_override {
        Some(directory) => directory.into(),
        None => app.path().app_local_data_dir()?.join("restic"),
    };
    let core = App::open(directory.clone())?;
    // Only notify about work completed after startup.
    let mut notification_cursor = None;
    core.notification_states(&mut notification_cursor);
    let settings = settings::Service::new(
        Box::new(settings::FilePreferences(
            directory.join("desktop-settings.json"),
        )),
        autostart,
    );
    app.manage(Desktop {
        core: core.clone(),
        settings,
        tray_created: AtomicBool::new(false),
        hidden_to_tray: AtomicBool::new(false),
        shutting_down: AtomicBool::new(false),
        drained: AtomicBool::new(false),
        file_dialog_open: Arc::new(AtomicBool::new(false)),
    });

    if let Err(error) = tray::setup(app) {
        eprintln!("Tray unavailable: {error}");
    }
    let handle = app.handle();
    let state = handle.state::<Desktop>();
    let tray_available = tray::available(handle);
    let start_hidden = match state.settings.get(tray_available) {
        Ok(settings) => settings.start_hidden(login_launch, manual_reopen.load(Ordering::Acquire)),
        Err(error) => {
            eprintln!("Opening the main window: {error}");
            false
        }
    };
    if start_hidden {
        state.hidden_to_tray.store(true, Ordering::Release);
    } else {
        show(handle);
    }

    let handle = handle.clone();
    std::thread::spawn(move || {
        let state = handle.state::<Desktop>();
        while !state.shutting_down.load(Ordering::Acquire) {
            if state.hidden_to_tray.load(Ordering::Acquire) && !tray::available(&handle) {
                show(&handle);
            }
            core.tick();
            for result in core.notification_states(&mut notification_cursor) {
                tray::notify(&result);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
    Ok(())
}

fn main() {
    let directory_override = std::env::var_os("YSYNK_RESTIC_DATA_DIR");
    let login_launch = settings::is_login_launch(std::env::args_os());
    let manual_reopen = Arc::new(AtomicBool::new(false));
    let activation = manual_reopen.clone();
    let mut builder = tauri::Builder::default();
    // Register first. Overrides intentionally use only the core data-dir lock,
    // not the production app's global activation identity.
    if directory_override.is_none() {
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
    if directory_override.is_none() {
        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .app_name("ysynk")
                .build(),
        );
    }
    let application = builder
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| setup(app, directory_override, login_launch, &manual_reopen))
        .invoke_handler(tauri::generate_handler![
            commands::request,
            commands::select_restic_executable,
            commands::export_repository_password,
            commands::get_desktop_settings,
            commands::save_desktop_settings,
            commands::quit,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Desktop>();
                if state.settings.preferences().hide_to_tray
                    && tray::available(window.app_handle())
                    && window.hide().is_ok()
                {
                    state.hidden_to_tray.store(true, Ordering::Release);
                    api.prevent_close();
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
