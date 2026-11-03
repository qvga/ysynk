/// Creation of an appindicator is not proof that a tray host is registered.
/// Suppress D-Bus activation: checking availability must not start a watcher.
#[cfg(target_os = "linux")]
pub fn available() -> bool {
    use dbus::{arg::Variant, blocking::BlockingSender};
    let Ok(connection) = dbus::blocking::Connection::new_session() else {
        return false;
    };
    let Ok(message) = dbus::Message::new_method_call(
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.freedesktop.DBus.Properties",
        "Get",
    ) else {
        return false;
    };
    let mut message = message.append2(
        "org.kde.StatusNotifierWatcher",
        "IsStatusNotifierHostRegistered",
    );
    message.set_auto_start(false);
    connection
        .send_with_reply_and_block(message, std::time::Duration::from_millis(500))
        .ok()
        .and_then(|reply| reply.read1::<Variant<bool>>().ok())
        .is_some_and(|value| value.0)
}
#[cfg(not(target_os = "linux"))]
pub fn available() -> bool {
    true
}
