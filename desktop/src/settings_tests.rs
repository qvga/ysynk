use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

#[derive(Default)]
struct Saved {
    preferences: Preferences,
    fail: bool,
}
struct FakePreferences(Arc<Mutex<Saved>>);
impl PreferenceStore for FakePreferences {
    fn load(&self) -> Result<Preferences> {
        Ok(self.0.lock().unwrap().preferences)
    }
    fn save(&self, preferences: Preferences) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.fail {
            return Err("fixture write failure".into());
        }
        state.preferences = preferences;
        Ok(())
    }
}
#[derive(Default)]
struct Login {
    enabled: bool,
    changes: Vec<bool>,
    outcomes: VecDeque<(bool, bool)>, // (commit, fail)
    pause: Option<(Arc<Barrier>, Arc<Barrier>)>,
}
struct FakeLogin(Arc<Mutex<Login>>);
impl Autostart for FakeLogin {
    fn supported(&self) -> bool {
        true
    }
    fn enabled(&self) -> Result<bool> {
        Ok(self.0.lock().unwrap().enabled)
    }
    fn set_enabled(&self, enabled: bool) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        let (commit, fail) = state.outcomes.pop_front().unwrap_or((true, false));
        if commit {
            state.enabled = enabled;
        }
        state.changes.push(enabled);
        let pause = state.pause.take();
        drop(state);
        if let Some((entered, resume)) = pause {
            entered.wait();
            resume.wait();
        }
        if fail {
            Err("fixture registration failure".into())
        } else {
            Ok(())
        }
    }
}
fn input(start_at_login: bool, start_minimized: bool, hide_to_tray: bool) -> Input {
    Input {
        start_at_login,
        start_minimized,
        hide_to_tray,
    }
}

#[test]
fn only_login_launches_with_valid_tray_preferences_start_hidden() {
    for login in [false, true] {
        for minimized in [false, true] {
            for hide in [false, true] {
                for tray in [false, true] {
                    for manual in [false, true] {
                        assert_eq!(
                            start_hidden(
                                login,
                                Preferences {
                                    start_minimized: minimized,
                                    hide_to_tray: hide
                                },
                                tray,
                                manual
                            ),
                            login && minimized && hide && tray && !manual
                        );
                    }
                }
            }
        }
    }
    assert!(is_login_launch(["ysynk", "--autostart"]));
    assert!(!is_login_launch(["ysynk", "--autostart=false"]));
    assert!(!is_login_launch(["--autostart"])); // executable is never an argument
    assert!(safe_executable(Path::new(if cfg!(windows) {
        "C:\\ysynk\\ysynk.exe"
    } else {
        "/opt/ysynk/ysynk"
    })));
    for unsafe_path in [
        "relative",
        "/opt/a b/ysynk",
        "/opt/a\"b/ysynk",
        "/opt/a%b/ysynk",
        "/opt/a&b/ysynk",
        "/opt/a\nb/ysynk",
    ] {
        assert!(!safe_executable(Path::new(unsafe_path)));
    }
}

#[test]
fn desktop_settings_transactions_restore_registration_and_serialize_reads_and_saves() {
    let saved = Arc::new(Mutex::new(Saved::default()));
    let login = Arc::new(Mutex::new(Login::default()));
    let service = Arc::new(Service::new(
        Box::new(FakePreferences(saved.clone())),
        Box::new(FakeLogin(login.clone())),
    ));
    assert!(!service.get(true).unwrap().start_at_login);
    assert!(service.save(input(false, true, false), true).is_err());
    assert!(service.save(input(false, true, true), false).is_err());
    assert!(login.lock().unwrap().changes.is_empty());
    let actual = service.save(input(true, true, true), true).unwrap();
    assert!(actual.start_at_login && actual.start_minimized && actual.hide_to_tray);
    // A disappearing host must not prevent unrelated edits or disabling.
    assert!(service.save(input(false, true, true), false).is_ok());
    assert!(service.save(input(false, false, false), false).is_ok());
    assert_eq!(service.preferences(), Preferences::default());
    saved.lock().unwrap().fail = true;
    assert!(
        service
            .save(input(true, false, false), true)
            .unwrap_err()
            .contains("Could not save")
    );
    assert!(!login.lock().unwrap().enabled);
    assert_eq!(saved.lock().unwrap().preferences, Preferences::default());
    assert_eq!(service.preferences(), Preferences::default());
    saved.lock().unwrap().fail = false;
    // An OS API can commit and then report an error. Restore even then, and
    // report a failed rollback without hiding the actual native state on reload.
    login.lock().unwrap().outcomes.push_back((true, true));
    assert!(service.save(input(true, false, false), true).is_err());
    assert!(!service.get(true).unwrap().start_at_login);
    login
        .lock()
        .unwrap()
        .outcomes
        .extend([(true, true), (false, true)]);
    assert!(
        service
            .save(input(true, false, false), true)
            .unwrap_err()
            .contains("may have changed")
    );
    assert!(service.get(true).unwrap().start_at_login);
    assert_eq!(service.preferences(), Preferences::default());
    service.save(input(false, false, false), true).unwrap();

    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    login.lock().unwrap().pause = Some((entered.clone(), resume.clone()));
    let first = service.clone();
    let writer = thread::spawn(move || first.save(input(true, false, false), true).unwrap());
    entered.wait(); // OS state is true, but preference publication is pending.
    let (read_tx, read_rx) = mpsc::channel();
    let reader = service.clone();
    let read_thread = thread::spawn(move || read_tx.send(reader.get(true).unwrap()).unwrap());
    let (save_tx, save_rx) = mpsc::channel();
    let second = service.clone();
    let second_thread = thread::spawn(move || {
        save_tx
            .send(second.save(input(false, false, true), true).unwrap())
            .unwrap()
    });
    thread::sleep(Duration::from_millis(30));
    assert!(read_rx.try_recv().is_err());
    assert!(save_rx.try_recv().is_err());
    resume.wait();
    writer.join().unwrap();
    let observed = read_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(observed.start_at_login != observed.hide_to_tray); // one complete transaction, never a partial state
    let final_save = save_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!final_save.start_at_login && final_save.hide_to_tray);
    read_thread.join().unwrap();
    second_thread.join().unwrap();
    assert!(!login.lock().unwrap().enabled);
    assert_eq!(service.preferences(), saved.lock().unwrap().preferences);
    service.close();
    assert!(service.get(true).is_err());
    assert!(service.save(input(true, false, false), true).is_err());
    assert!(!login.lock().unwrap().enabled);
}

#[cfg(target_os = "linux")]
#[test]
fn isolated_xdg_registration_and_preferences_preserve_real_state_and_override_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("isolated-xdg-config");
    let autostart = XdgAutostart {
        directory: Some(config.clone()),
        executable: PathBuf::from("/opt/ysynk/ysynk"),
    };
    assert!(!autostart.enabled().unwrap());
    autostart.set_enabled(true).unwrap();
    assert!(autostart.enabled().unwrap());
    let registration = config.join("autostart/ysynk.desktop");
    let original = fs::read(&registration).unwrap();
    assert!(String::from_utf8_lossy(&original).contains("Exec=/opt/ysynk/ysynk --autostart\n"));
    fs::write(
        &registration,
        [original.clone(), b"Hidden=true\n".to_vec()].concat(),
    )
    .unwrap();
    assert!(!autostart.enabled().unwrap());
    fs::write(
        &registration,
        [
            original.clone(),
            b"X-GNOME-Autostart-enabled=false\n".to_vec(),
        ]
        .concat(),
    )
    .unwrap();
    assert!(!autostart.enabled().unwrap());
    fs::write(&registration, &original).unwrap();
    // A native override adapter never reads or modifies the normal login
    // registration, including when saving a request that disables autostart.
    let local_file = FilePreferences(root.path().join("desktop-settings.json"));
    let local = Service::new(Box::new(local_file), Box::new(DisabledAutostart));
    let actual = local.get(true).unwrap();
    assert!(!actual.start_at_login && !actual.autostart_supported);
    local.save(input(false, true, true), true).unwrap();
    assert!(local.save(input(true, true, true), true).is_err());
    assert_eq!(fs::read(&registration).unwrap(), original);
    let prefs = FilePreferences(root.path().join("desktop-settings.json"));
    assert_eq!(
        prefs.load().unwrap(),
        Preferences {
            start_minimized: true,
            hide_to_tray: true
        }
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&prefs.0).unwrap())
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        2
    );
    // Restore the exact old entry after a preference-write failure, not a new
    // executable registration (the installed path may differ from this launch).
    let bad_preferences = FilePreferences(root.path().join("missing-parent/settings.json"));
    let service = Service::new(
        Box::new(bad_preferences),
        Box::new(XdgAutostart {
            directory: Some(config.clone()),
            executable: PathBuf::from("/opt/other/ysynk"),
        }),
    );
    assert!(service.save(input(false, false, false), true).is_err());
    assert_eq!(fs::read(&registration).unwrap(), original);
    for unsafe_path in [
        "/opt/a b/ysynk",
        "/opt/a%b/ysynk",
        "/opt/a\\b/ysynk",
        "/opt/a\"b/ysynk",
    ] {
        let untouched = root.path().join("unsafe-not-created");
        let adapter = XdgAutostart {
            directory: Some(untouched.clone()),
            executable: PathBuf::from(unsafe_path),
        };
        assert!(!adapter.supported());
        assert!(adapter.set_enabled(true).is_err());
        assert!(!untouched.exists());
    }
    autostart.set_enabled(false).unwrap();
    assert!(!autostart.enabled().unwrap());
    assert!(!registration.exists());
    fs::write(&prefs.0, vec![b'x'; 65537]).unwrap();
    assert!(prefs.load().is_err());
    let broken = Service::new(Box::new(prefs), Box::new(DisabledAutostart));
    assert!(broken.get(true).is_err());
    assert_eq!(broken.preferences(), Preferences::default());
}
