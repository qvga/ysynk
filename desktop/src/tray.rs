use crate::{Desktop, begin_shutdown, show, tray_host};
use std::sync::atomic::Ordering;
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub fn available(handle: &tauri::AppHandle) -> bool {
    handle
        .state::<Desktop>()
        .tray_created
        .load(Ordering::Acquire)
        && tray_host::available()
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open ysynk", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit ysynk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let icon = app
        .default_window_icon()
        .ok_or_else(|| tauri::Error::AssetNotFound("application icon".into()))?;
    TrayIconBuilder::new()
        .icon(icon.clone())
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
        .store(true, Ordering::Release);
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn notify(state: &str) {
    let Ok(connection) = dbus::blocking::Connection::new_session() else {
        return;
    };
    let body = match state {
        "success" => "The operation completed. Open ysynk to review the results.",
        "partial" => {
            "Some data or destinations were not backed up. Open ysynk to review the results."
        }
        "cancelled" => "The operation was cancelled. Open ysynk to review individual results.",
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
                "ysynk-desktop",
                "ysynk backup activity",
                body,
                Vec::<String>::new(),
                dbus::arg::PropMap::new(),
                -1_i32,
            ),
        );
}

#[cfg(not(target_os = "linux"))]
pub fn notify(_state: &str) {}
