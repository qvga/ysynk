use super::*;
use crate::credentials;
use std::{
    sync::Barrier,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct FakeState {
    entries: HashMap<String, Vec<u8>>,
    read_error: Option<&'static str>,
    write_mode: u8,
    reads: usize,
    writes: usize,
}
#[derive(Default)]
struct FakeStore {
    state: Mutex<FakeState>,
    pause: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
}
impl Store for FakeStore {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let pause = self.pause.lock().unwrap().take();
        if let Some((entered, resume)) = pause {
            entered.wait();
            resume.wait();
        }
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        if let Some(error) = state.read_error {
            bail!(error);
        }
        Ok(state.entries.get(account).cloned().map(Zeroizing::new))
    }
    fn create(&self, account: &str, bytes: &[u8]) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.writes += 1;
        ensure!(
            !state.entries.contains_key(account),
            "fixture refuses overwrite"
        );
        if state.write_mode != 1 {
            state.entries.insert(account.into(), bytes.to_vec());
        }
        ensure!(
            state.write_mode == 0,
            "Could not save repository credential"
        );
        Ok(())
    }
}
fn open(directory: PathBuf, store: Arc<FakeStore>, locks: PathBuf, c: &Config) -> Result<Arc<App>> {
    fs::create_dir_all(&directory)?;
    config::write(
        &directory.join("config.json"),
        &serde_json::to_vec(c)?,
        false,
    )?;
    App::open_inner(directory, store, locks)
}
fn wait(app: &Arc<App>) -> Result<Job> {
    let start = Instant::now();
    while app.is_busy() {
        ensure!(
            start.elapsed() < Duration::from_secs(60),
            "test job timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
    Ok(app.state.lock().unwrap().history[0].clone())
}
fn finish(app: &Arc<App>, request: Request) -> Result<Job> {
    app.request(request)?;
    wait(app)
}
fn success(app: &Arc<App>, request: Request) -> Result<Job> {
    let job = finish(app, request)?;
    ensure!(job.state == "success", "job {}: {}", job.kind, job.message);
    Ok(job)
}
fn repository(dir: &Path) -> Repository {
    Repository {
        id: "fixture".into(),
        label: "Disposable fixture".into(),
        location: dir.join("repo").to_str().unwrap().into(),
    }
}
fn initialize(repo: &Repository) -> Request {
    Request::Initialize {
        repository_id: repo.id.clone(),
    }
}
fn check(repo: &Repository) -> Request {
    Request::Check {
        repository_id: repo.id.clone(),
    }
}

#[test]
fn audit_save_rejects_ambiguous_credentials_without_exposing_them() -> Result<()> {
    for location in [
        "rest:https://alice:SENTINEL@host/repo",
        "rest:https://alice@example.com:SENTINEL@host/repo",
        "sftp:alice:SENTINEL@host:/repo",
        "sftp://alice@example.com:SENTINEL@host/repo",
    ] {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let app = open(
            dir.path().join("app"),
            store.clone(),
            dir.path().join("locks"),
            &Config::default(),
        )?;
        let mut c = Config::default();
        c.repositories.push(Repository {
            id: "new".into(),
            label: "Fixture".into(),
            location: location.into(),
        });
        let result = app.request(Request::SaveConfig { config: c });
        assert!(result.is_err(), "accepted ambiguous userinfo");
        assert!(!result.unwrap_err().to_string().contains("SENTINEL"));
        assert!(
            !app.request(Request::Status)?
                .to_string()
                .contains("SENTINEL")
        );
        assert!(!fs::read_to_string(app.directory.join("config.json"))?.contains("SENTINEL"));
        assert!(!fs::read_to_string(app.directory.join("state.json"))?.contains("SENTINEL"));
        assert_eq!(store.state.lock().unwrap().reads, 0);
    }
    Ok(())
}

#[test]
fn audit_save_preserves_username_ipv6_and_path_separators() -> Result<()> {
    for location in [
        "rest:https://alice@host/repo@archive",
        "rest:https://alice@example.com@host/repo@archive",
        "rest:https://alice@[::1]:8000/repo@archive",
        "rest:https://[::1]:8000/repo@archive",
        "sftp:alice@host:/repo@archive",
        "sftp:alice@[::1]:/repo@archive",
        "sftp:[::1]:/repo@archive",
        "sftp://alice@[::1]:22/repo@archive",
        "sftp://[::1]:22/repo@archive",
    ] {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let app = open(
            dir.path().join("app"),
            store.clone(),
            dir.path().join("locks"),
            &Config::default(),
        )?;
        let c = Config {
            repositories: vec![Repository {
                id: "valid".into(),
                label: "Fixture".into(),
                location: location.into(),
            }],
            ..Config::default()
        };
        app.request(Request::SaveConfig { config: c })?;
        assert_eq!(
            app.request(Request::Status)?["config"]["repositories"][0]["location"],
            location
        );
        assert_eq!(store.state.lock().unwrap().reads, 0);
    }
    Ok(())
}

#[test]
fn audit_scheduler_selects_oldest_deadline_after_each_completion() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let app = open(
        dir.path().join("app"),
        Arc::new(FakeStore::default()),
        dir.path().join("locks"),
        &Config::default(),
    )?;
    let mut state = app.state.lock().unwrap();
    state.config.profiles = [("first", 1), ("second", 3), ("third", 2)]
        .into_iter()
        .map(|(id, schedule_minutes)| Profile {
            id: id.into(),
            label: id.into(),
            sources: vec![],
            excludes: vec![],
            repository_ids: vec![],
            schedule_minutes,
            retention: Retention {
                keep_last: 1,
                keep_daily: 0,
                keep_weekly: 0,
                keep_monthly: 0,
            },
        })
        .collect();
    // Deadlines are 960, 880, and 920, in a different order from config.
    state.attempts = [
        ("first".into(), 900),
        ("second".into(), 700),
        ("third".into(), 800),
    ]
    .into();
    for (clock, expected, completed) in [
        (1000, "second", 1100),
        (1100, "third", 1200),
        (1200, "first", 1300),
        (1300, "second", 1400),
    ] {
        assert_eq!(due_profile(&state, clock).as_deref(), Some(expected));
        state.attempts.insert(expected.into(), completed);
    }
    state.attempts.values_mut().for_each(|t| *t = u64::MAX);
    assert_eq!(due_profile(&state, u64::MAX - 1), None);
    Ok(())
}

#[test]
fn existing_sftp_locators_keep_their_exact_keychain_binding_on_load_and_save() -> Result<()> {
    for location in [
        "sftp:alice@host:repo@archive",
        "sftp:alice@example.com@host:/repo",
        "sftp:alice@host:repo@archive:old",
        "sftp:alice@host:/repo@archive",
        "rest:https://alice@example.com@host/repo",
    ] {
        let directory = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let repo = Repository {
            location: location.into(),
            ..repository(directory.path())
        };
        let record = credentials::Record::generate(&repo);
        store
            .state
            .lock()
            .unwrap()
            .entries
            .insert(repo.id.clone(), record.bytes()?.to_vec());
        let configuration = Config {
            repositories: vec![repo.clone()],
            ..Config::default()
        };
        // The file represents an existing installation. Loading it must not
        // require rewriting the exact locator stored in its keychain record.
        let app = open(
            directory.path().join("app"),
            store.clone(),
            directory.path().join("locks"),
            &configuration,
        )?;
        let loaded = credentials::required(
            store.as_ref(),
            &app.state.lock().unwrap().config.repositories[0],
        )?;
        assert!(loaded.password == record.password);
        let status = app.request(Request::SaveConfig {
            config: configuration,
        })?;
        assert_eq!(status["config"]["repositories"][0]["location"], location);
        assert_eq!(store.state.lock().unwrap().writes, 0);
        app.shutdown();
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn only_native_selection_can_change_the_engine() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &Config::default(),
    )?;
    let binary = dir.path().join("restic-custom");
    fs::write(&binary, b"")?;
    let config = Config {
        restic_path: binary.to_str().unwrap().into(),
        ..Config::default()
    };
    assert!(app.request(Request::SaveConfig { config }).is_err());
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o600))?;
    assert!(app.set_restic_executable(0, &binary).is_err());
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
    let alias = dir.path().join("selected");
    symlink(&binary, &alias)?;
    assert!(app.set_restic_executable(1, &alias).is_err());
    let updated = app.set_restic_executable(0, &alias)?;
    assert_eq!(updated["config"]["restic_path"], binary.to_str().unwrap());
    assert_eq!(updated["config"]["revision"], 1);
    assert!(updated["restic_version"].is_null());
    assert!(updated["setup_error"].as_str().unwrap().contains("restart"));
    let persisted: Config = serde_json::from_slice(&fs::read(app.directory.join("config.json"))?)?;
    assert_eq!(persisted.restic_path, binary.to_str().unwrap());
    assert!(
        app.request(Request::SaveConfig { config: persisted })
            .is_ok()
    );
    assert_eq!(store.state.lock().unwrap().reads, 0);
    Ok(())
}

#[test]
fn audit_write_failures_block_admission_and_clear_previews() -> Result<()> {
    for stage in ["publish", "directory_sync"] {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let app = open(
            dir.path().join("app"),
            store.clone(),
            dir.path().join("locks"),
            &Config::default(),
        )?;
        app.state.lock().unwrap().setup_error = None;
        app.state.lock().unwrap().previews.insert(
            "stale".into(),
            Preview {
                expires: u64::MAX,
                revision: 0,
                repository: repository(dir.path()),
                profile_id: "profile".into(),
                repository_identity: "a".repeat(64),
                remove: vec![],
                survivors: vec![],
            },
        );
        let c = Config::default();
        config::WRITE_FAILURE.with(|fault| fault.set(Some(stage)));
        assert!(app.request(Request::SaveConfig { config: c }).is_err());
        let status = app.request(Request::Status)?;
        assert!(
            status["setup_error"]
                .as_str()
                .is_some_and(|s| s.contains("restart"))
        );
        assert!(app.state.lock().unwrap().previews.is_empty());
        assert!(
            app.request(Request::Backup {
                profile_id: "missing".into()
            })
            .is_err()
        );
        assert_eq!(store.state.lock().unwrap().reads, 0);
        assert_eq!(app.state.lock().unwrap().config.revision, 0);
        let disk: Config = serde_json::from_slice(&fs::read(app.directory.join("config.json"))?)?;
        assert_eq!(disk.revision, u64::from(stage == "directory_sync"));
        assert!(
            app.request(Request::SaveConfig {
                config: Config::default()
            })
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn audit_failed_admission_preserves_attempts_and_blocks_retry() -> Result<()> {
    for stage in ["publish", "directory_sync"] {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let repo = repository(dir.path());
        let c = Config {
            repositories: vec![repo.clone()],
            profiles: vec![Profile {
                id: "profile".into(),
                label: "Fixture".into(),
                sources: vec![dir.path().join("source").to_str().unwrap().into()],
                excludes: vec![],
                repository_ids: vec![repo.id],
                schedule_minutes: 1,
                retention: Retention {
                    keep_last: 1,
                    keep_daily: 0,
                    keep_weekly: 0,
                    keep_monthly: 0,
                },
            }],
            ..Config::default()
        };
        let app = open(
            dir.path().join("app"),
            store.clone(),
            dir.path().join("locks"),
            &c,
        )?;
        {
            let mut state = app.state.lock().unwrap();
            state.setup_error = None;
            state.attempts.insert("profile".into(), 17);
        }
        config::WRITE_FAILURE.with(|fault| fault.set(Some(stage)));
        assert!(
            app.request(Request::Backup {
                profile_id: "profile".into()
            })
            .is_err()
        );
        {
            let state = app.state.lock().unwrap();
            assert_eq!(state.attempts["profile"], 17);
            assert!(state.active.is_none());
            assert!(
                state
                    .setup_error
                    .as_ref()
                    .is_some_and(|s| s.contains("restart"))
            );
        }
        let disk: Persisted = serde_json::from_slice(&fs::read(app.directory.join("state.json"))?)?;
        // A failed directory sync may have committed admission. Do not undo
        // publication by pathname; startup treats that saved job as interrupted.
        assert_eq!(disk.active.is_some(), stage == "directory_sync");
        if stage == "directory_sync" {
            assert_ne!(disk.attempts["profile"], 17);
        }
        app.tick();
        assert!(
            app.request(Request::Backup {
                profile_id: "profile".into()
            })
            .is_err()
        );
        assert!(app.workers.lock().unwrap().is_empty());
        assert_eq!(store.state.lock().unwrap().reads, 0);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn audit_export_rejects_untrusted_parent_before_reading_credentials() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(dir.path());
    let c = Config {
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    store.state.lock().unwrap().entries.insert(
        repo.id.clone(),
        credentials::Record::generate(&repo).bytes()?.to_vec(),
    );
    let parent = dir.path().join("shared");
    fs::create_dir(&parent)?;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777))?;
    assert!(
        app.export_repository_password(&repo.id, 0, parent.join("SENTINEL"))
            .is_err()
    );
    assert_eq!(store.state.lock().unwrap().reads, 0);
    assert_eq!(fs::read_dir(parent)?.count(), 0);
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn audit_export_pins_parent_across_ancestor_rename_and_symlink_replacement() -> Result<()> {
    use std::os::unix::{fs::PermissionsExt, fs::symlink};
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(dir.path());
    let c = Config {
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    let record = credentials::Record::generate(&repo);
    let expected = format!("{}\n", record.password);
    store
        .state
        .lock()
        .unwrap()
        .entries
        .insert(repo.id.clone(), record.bytes()?.to_vec());
    let ancestor = dir.path().join("ancestor");
    fs::create_dir_all(ancestor.join("private"))?;
    fs::set_permissions(ancestor.join("private"), fs::Permissions::from_mode(0o700))?;
    let alternate = dir.path().join("alternate");
    fs::create_dir(&alternate)?;
    symlink(&alternate, dir.path().join("link"))?;
    let error = app
        .export_repository_password(&repo.id, 0, dir.path().join("link/SENTINEL"))
        .unwrap_err()
        .to_string();
    assert!(!error.contains("SENTINEL"));
    assert_eq!(store.state.lock().unwrap().reads, 0);
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *store.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
    let task_app = app.clone();
    let target = ancestor.join("private/password");
    let task = thread::spawn(move || task_app.export_repository_password(&repo.id, 0, target));
    entered.wait();
    let moved = dir.path().join("moved");
    fs::rename(&ancestor, &moved)?;
    fs::create_dir(alternate.join("private"))?;
    symlink(&alternate, &ancestor)?;
    resume.wait();
    task.join().unwrap()?;
    assert_eq!(
        fs::read(moved.join("private/password"))?,
        expected.as_bytes()
    );
    assert_eq!(
        fs::metadata(moved.join("private/password"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(moved.join("private"))?.count(), 1);
    assert_eq!(fs::read_dir(alternate.join("private"))?.count(), 0);
    assert!(
        !app.request(Request::Status)?
            .to_string()
            .contains(expected.trim())
    );
    Ok(())
}

#[test]
fn audit_completion_write_failure_preserves_outcome_and_blocks_jobs() -> Result<()> {
    for stage in ["publish", "directory_sync"] {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(FakeStore::default());
        let app = open(
            dir.path().join("app"),
            store.clone(),
            dir.path().join("locks"),
            &Config::default(),
        )?;
        {
            let mut state = app.state.lock().unwrap();
            state.setup_error = None;
            state.active = Some(serde_json::from_value(json!({
                "id":"job", "kind":"backup", "label":"backup", "state":"running", "started_at":1,
                "profile_id":"profile", "message":"Running", "destinations":[]
            }))?);
        }
        config::WRITE_FAILURE.with(|fault| fault.set(Some(stage)));
        app.complete(Ok(json!({"completed":true})));
        let state = app.state.lock().unwrap();
        assert!(state.active.is_none());
        assert_eq!(state.history[0].state, "success");
        assert!(state.attempts["profile"] > 0);
        assert!(state.persistence_failed);
        assert!(state.setup_error.as_ref().unwrap().contains("restart"));
        drop(state);
        app.tick();
        assert!(
            app.request(Request::Backup {
                profile_id: "profile".into()
            })
            .is_err()
        );
        assert!(app.workers.lock().unwrap().is_empty());
        assert_eq!(store.state.lock().unwrap().reads, 0);
    }
    Ok(())
}

#[test]
fn password_export_writes_only_a_new_private_file_and_reserves_admission() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(dir.path());
    let source = dir.path().join("source");
    let recovery = dir.path().join("private-recovery");
    fs::create_dir(&source)?;
    fs::create_dir(&recovery)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&recovery, fs::Permissions::from_mode(0o700))?;
    }
    fs::create_dir(&repo.location)?;
    let c = Config {
        repositories: vec![repo.clone()],
        profiles: vec![Profile {
            id: "profile".into(),
            label: "Fixture".into(),
            sources: vec![source.to_str().unwrap().into()],
            excludes: vec![],
            repository_ids: vec![repo.id.clone()],
            schedule_minutes: 0,
            retention: Retention {
                keep_last: 1,
                keep_daily: 0,
                keep_weekly: 0,
                keep_monthly: 0,
            },
        }],
        ..Config::default()
    };
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    let original = credentials::Record::generate(&repo).bytes()?.to_vec();
    let expected: Value = serde_json::from_slice(&original)?;
    let password = expected["password"].as_str().unwrap();
    store
        .state
        .lock()
        .unwrap()
        .entries
        .insert(repo.id.clone(), original.clone());
    let target = recovery.join("password.txt");
    // Native preflight is credential-free; closing the Save dialog can return
    // without invoking the export method or accessing the keychain at all.
    app.validate_repository_password_export(&repo.id, 0)?;
    assert_eq!(store.state.lock().unwrap().reads, 0);
    assert!(
        app.validate_repository_password_export(&repo.id, 1)
            .is_err()
    );
    assert!(
        app.export_repository_password(&repo.id, 1, target.clone())
            .is_err()
    );
    assert!(
        app.export_repository_password("unknown", 0, target.clone())
            .is_err()
    );
    assert_eq!(store.state.lock().unwrap().reads, 0);
    for forbidden in [
        app.directory.join("secret.txt"),
        app.directory.join("cache/secret.txt"),
        source.join("secret.txt"),
        Path::new(&repo.location).join("secret.txt"),
    ] {
        assert!(
            app.export_repository_password(&repo.id, 0, forbidden.clone())
                .is_err()
        );
        assert!(!forbidden.exists());
    }
    let existing = recovery.join("existing.txt");
    fs::write(&existing, b"unrelated")?;
    assert!(
        app.export_repository_password(&repo.id, 0, existing.clone())
            .is_err()
    );
    assert_eq!(fs::read(&existing)?, b"unrelated");
    #[cfg(unix)]
    {
        let link = recovery.join("existing-link");
        let dangling = recovery.join("dangling-link");
        let parent_alias = recovery.join("source-alias");
        std::os::unix::fs::symlink(&existing, &link)?;
        std::os::unix::fs::symlink(recovery.join("absent"), &dangling)?;
        std::os::unix::fs::symlink(&source, &parent_alias)?;
        for forbidden in [
            link.clone(),
            dangling.clone(),
            parent_alias.join("secret.txt"),
        ] {
            assert!(
                app.export_repository_password(&repo.id, 0, forbidden)
                    .is_err()
            );
        }
        assert_eq!(fs::read_link(link)?, existing);
        assert_eq!(fs::read_link(dangling)?, recovery.join("absent"));
        assert!(!source.join("secret.txt").exists());
    }
    assert_eq!(store.state.lock().unwrap().reads, 0);
    store.state.lock().unwrap().entries.clear();
    assert!(
        app.export_repository_password(&repo.id, 0, target.clone())
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    for error in [credentials::LOCKED, credentials::UNAVAILABLE] {
        store.state.lock().unwrap().read_error = Some(error);
        assert_eq!(
            app.export_repository_password(&repo.id, 0, target.clone())
                .unwrap_err()
                .to_string(),
            error
        );
    }
    store.state.lock().unwrap().read_error = None;
    let moved = Repository {
        location: dir.path().join("other").to_str().unwrap().into(),
        ..repo.clone()
    };
    store.state.lock().unwrap().entries.insert(
        repo.id.clone(),
        credentials::Record::generate(&moved).bytes()?.to_vec(),
    );
    assert!(
        app.export_repository_password(&repo.id, 0, target.clone())
            .unwrap_err()
            .to_string()
            .contains("another location")
    );
    assert!(!target.exists());
    store
        .state
        .lock()
        .unwrap()
        .entries
        .insert(repo.id.clone(), original.clone());
    app.export_repository_password(&repo.id, 0, target.clone())?;
    assert_eq!(fs::read(&target)?, format!("{password}\n").as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o600);
    }
    assert!(
        app.export_repository_password(&repo.id, 0, target.clone())
            .is_err()
    );
    assert_eq!(fs::read(&target)?, format!("{password}\n").as_bytes());
    assert_eq!(store.state.lock().unwrap().writes, 0);
    assert_eq!(store.state.lock().unwrap().entries[&repo.id], original);
    assert!(!app.is_busy());

    // Hold the fake keychain call: status remains responsive, but no new job,
    // config save, or second export can enter until the reservation is dropped.
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *store.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
    let collision = recovery.join("collision.txt");
    let task_app = app.clone();
    let task_target = collision.clone();
    let task_id = repo.id.clone();
    let task = thread::spawn(move || task_app.export_repository_password(&task_id, 0, task_target));
    entered.wait();
    let status = app.request(Request::Status)?;
    assert!(status["active"].is_null());
    assert!(status["history"].as_array().unwrap().is_empty());
    assert!(app.is_busy());
    assert!(app.request(check(&repo)).is_err());
    assert!(
        app.request(Request::SaveConfig { config: c.clone() })
            .is_err()
    );
    assert!(
        app.export_repository_password(&repo.id, 0, recovery.join("second.txt"))
            .is_err()
    );
    // An unrelated file appears after initial validation. Publishing must not
    // overwrite it; the staged private file is removed on the failure path.
    fs::write(&collision, b"keep this file")?;
    let entries_before = fs::read_dir(&recovery)?.count();
    resume.wait();
    assert!(task.join().unwrap().is_err());
    assert_eq!(fs::read(&collision)?, b"keep this file");
    assert_eq!(fs::read_dir(&recovery)?.count(), entries_before);
    assert!(!app.is_busy());

    // Simulate a stale config after credential retrieval. Normal save requests
    // cannot do this during a reservation, but the final gate still refuses it.
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *store.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
    let stale_target = recovery.join("stale.txt");
    let task_app = app.clone();
    let task_target = stale_target.clone();
    let task_id = repo.id.clone();
    let task = thread::spawn(move || task_app.export_repository_password(&task_id, 0, task_target));
    entered.wait();
    app.state.lock().unwrap().config.revision += 1;
    resume.wait();
    assert!(task.join().unwrap().is_err());
    assert!(!stale_target.exists());
    assert!(!app.is_busy());
    assert!(!app.request(Request::Status)?.to_string().contains(password));
    assert!(app.state.lock().unwrap().history.is_empty());
    assert_eq!(store.state.lock().unwrap().writes, 0);
    app.shutdown();
    assert!(
        app.export_repository_password(&repo.id, 1, recovery.join("closed.txt"))
            .is_err()
    );
    for name in ["config.json", "state.json"] {
        assert!(!fs::read_to_string(app.directory.join(name))?.contains(password));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn export_sync_failure_preserves_the_published_file_and_releases_admission() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(directory.path());
    let configuration = Config {
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let app = open(
        directory.path().join("app"),
        store.clone(),
        directory.path().join("locks"),
        &configuration,
    )?;
    let record = credentials::Record::generate(&repo);
    store
        .state
        .lock()
        .unwrap()
        .entries
        .insert(repo.id.clone(), record.bytes()?.to_vec());
    let target = directory.path().join("password");
    config::WRITE_FAILURE.with(|fault| fault.set(Some("export_directory_sync")));
    let result = app.export_repository_password(&repo.id, 0, target.clone());
    assert!(result.is_err());
    let error = result.unwrap_err().to_string();
    assert!(error.contains("durability"), "{error}");
    assert_eq!(
        fs::read(&target)?,
        format!("{}\n", record.password).as_bytes()
    );
    assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o600);
    assert!(!app.is_busy());
    assert!(!app.state.lock().unwrap().password_export);
    assert!(
        app.export_repository_password(&repo.id, 0, target.clone())
            .is_err()
    );
    assert_eq!(
        fs::read(target)?,
        format!("{}\n", record.password).as_bytes()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn only_the_latest_successful_retention_preview_remains_valid() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    fs::create_dir(directory.path().join("source"))?;
    fs::write(
        directory.path().join("source/document"),
        b"Disposable fixture",
    )?;
    let repositories: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| Repository {
            id: name.into(),
            label: name.into(),
            location: directory.path().join(name).to_str().unwrap().into(),
        })
        .collect();
    let configuration = Config {
        repositories: repositories.clone(),
        profiles: vec![Profile {
            id: "profile".into(),
            label: "Fixture".into(),
            sources: vec![directory.path().join("source").to_str().unwrap().into()],
            excludes: vec![],
            repository_ids: repositories.iter().map(|repo| repo.id.clone()).collect(),
            schedule_minutes: 0,
            retention: Retention {
                keep_last: 1,
                keep_daily: 0,
                keep_weekly: 0,
                keep_monthly: 0,
            },
        }],
        ..Config::default()
    };
    let app = open(
        directory.path().join("app"),
        store.clone(),
        directory.path().join("locks"),
        &configuration,
    )?;
    for repo in &repositories {
        success(&app, initialize(repo))?;
    }
    success(
        &app,
        Request::Backup {
            profile_id: "profile".into(),
        },
    )?;
    let preview = |repository_id: &str| Request::RetentionPreview {
        repository_id: repository_id.into(),
        profile_id: "profile".into(),
    };
    let mut tokens = vec![];
    // Replacement is global, even when a different repository is selected.
    for repository_id in ["first", "first", "second"] {
        let result = success(&app, preview(repository_id))?.result.unwrap();
        let token = result["token"].as_str().unwrap().to_owned();
        let state = app.state.lock().unwrap();
        assert_eq!(state.previews.len(), 1);
        assert!(state.previews.contains_key(&token));
        tokens.push(token);
    }
    // Failure must not invalidate an already completed, valid preview.
    store.state.lock().unwrap().read_error = Some(credentials::LOCKED);
    assert_eq!(finish(&app, preview("first"))?.state, "failed");
    store.state.lock().unwrap().read_error = None;
    assert!(app.state.lock().unwrap().previews.contains_key(&tokens[2]));
    let stale = finish(
        &app,
        Request::ApplyRetention {
            token: tokens[0].clone(),
            confirmation: "DELETE".into(),
        },
    )?;
    assert_eq!(stale.state, "failed");
    assert!(stale.message.contains("superseded"));
    success(
        &app,
        Request::ApplyRetention {
            token: tokens[2].clone(),
            confirmation: "DELETE".into(),
        },
    )?;
    assert!(app.state.lock().unwrap().previews.is_empty());
    app.shutdown();
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn retention_preview_of_an_empty_repository_has_no_candidates() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(directory.path());
    let configuration = Config {
        repositories: vec![repo.clone()],
        profiles: vec![Profile {
            id: "profile".into(),
            label: "Fixture".into(),
            sources: vec![directory.path().join("source").to_str().unwrap().into()],
            excludes: vec![],
            repository_ids: vec![repo.id.clone()],
            schedule_minutes: 0,
            retention: Retention {
                keep_last: 1,
                keep_daily: 0,
                keep_weekly: 0,
                keep_monthly: 0,
            },
        }],
        ..Config::default()
    };
    let app = open(
        directory.path().join("app"),
        store,
        directory.path().join("locks"),
        &configuration,
    )?;
    success(&app, initialize(&repo))?;
    let preview = success(
        &app,
        Request::RetentionPreview {
            repository_id: repo.id,
            profile_id: "profile".into(),
        },
    )?
    .result
    .unwrap();
    assert_eq!(preview["remove"], json!([]));
    assert_eq!(preview["keep_count"], 0);
    app.shutdown();
    Ok(())
}

#[test]
fn initialization_lock_is_shared_across_app_data_directories() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(dir.path());
    let c = Config {
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let locks = dir.path().join("user-keychain-locks");
    let one = open(dir.path().join("one"), store.clone(), locks.clone(), &c)?;
    let two = open(dir.path().join("two"), store.clone(), locks, &c)?;
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *store.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
    one.request(initialize(&repo))?;
    entered.wait();
    // The first worker holds the cross-process lock, not its state mutex.
    one.request(Request::Status)?;
    let second = finish(&two, initialize(&repo))?;
    assert!(second.message.contains("another app instance"));
    assert_eq!(store.state.lock().unwrap().writes, 0);
    resume.wait();
    assert_eq!(wait(&one)?.state, "success");
    let original = store.state.lock().unwrap().entries[&repo.id].clone();
    assert_eq!(
        success(&two, initialize(&repo))?.result.unwrap()["reconnected"],
        true
    );
    assert_eq!(store.state.lock().unwrap().entries[&repo.id], original);
    assert_eq!(store.state.lock().unwrap().writes, 1);
    one.shutdown();
    two.shutdown();
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn real_restic_preserves_literal_paths_and_applies_only_previewed_snapshots() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))?;
    let store = Arc::new(FakeStore::default());
    let source = dir.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("literal[*]?.txt"), b"first fixture")?;
    fs::write(source.join("literalx.txt"), b"must not restore")?;
    let repo = repository(dir.path());
    let profile = Profile {
        id: "fixture".into(),
        label: "Disposable fixture".into(),
        sources: vec![source.to_str().unwrap().into()],
        excludes: vec![],
        repository_ids: vec![repo.id.clone()],
        schedule_minutes: 0,
        retention: Retention {
            keep_last: 1,
            keep_daily: 0,
            keep_weekly: 0,
            keep_monthly: 0,
        },
    };
    let c = Config {
        repositories: vec![repo.clone()],
        profiles: vec![profile.clone()],
        ..Config::default()
    };
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    success(&app, initialize(&repo))?;
    let original = store.state.lock().unwrap().entries[&repo.id].clone();
    assert_eq!(
        success(&app, initialize(&repo))?.result.unwrap()["reconnected"],
        true
    );
    assert_eq!(store.state.lock().unwrap().entries[&repo.id], original);
    let backup = success(
        &app,
        Request::Backup {
            profile_id: profile.id.clone(),
        },
    )?;
    let progress = backup.progress.unwrap();
    assert_eq!(progress.percent, Some(100.0));
    assert_eq!(progress.files_done, 2);
    assert!(progress.bytes_done > 0);
    assert!(fs::read_dir(app.directory.join("cache"))?.next().is_some());
    fs::write(source.join("literal[*]?.txt"), b"second fixture")?;
    success(
        &app,
        Request::Backup {
            profile_id: profile.id.clone(),
        },
    )?;
    let snapshots = success(
        &app,
        Request::Snapshots {
            repository_id: repo.id.clone(),
        },
    )?
    .result
    .unwrap();
    assert_eq!(snapshots.as_array().unwrap().len(), 2);
    let snapshot_id = snapshots[0]["id"].as_str().unwrap().to_owned();
    let listing = success(
        &app,
        Request::ListFiles {
            repository_id: repo.id.clone(),
            snapshot_id: snapshot_id.clone(),
            path: source.to_str().unwrap().into(),
        },
    )?
    .result
    .unwrap();
    assert_eq!(listing.as_array().unwrap().len(), 2);
    let target = dir.path().join("restore");
    success(
        &app,
        Request::Restore {
            repository_id: repo.id.clone(),
            snapshot_id,
            paths: vec![source.join("literal[*]?.txt").to_str().unwrap().into()],
            target: target.to_str().unwrap().into(),
        },
    )?;
    let restored = target.join(source.strip_prefix("/")?);
    assert!(restored.join("literal[*]?.txt").is_file());
    assert!(!restored.join("literalx.txt").exists());
    assert!(private_restore_directory(&target).is_err());
    let symlink = dir.path().join("link");
    std::os::unix::fs::symlink(&source, &symlink)?;
    assert!(private_restore_directory(&symlink.join("unsafe")).is_err());
    success(
        &app,
        Request::Stats {
            repository_id: repo.id.clone(),
        },
    )?;
    success(&app, check(&repo))?;
    let preview = success(
        &app,
        Request::RetentionPreview {
            repository_id: repo.id.clone(),
            profile_id: profile.id.clone(),
        },
    )?
    .result
    .unwrap();
    assert_eq!(preview["keep_count"], 1);
    assert_eq!(preview["remove"].as_array().unwrap().len(), 1);
    let token = preview["token"].as_str().unwrap().to_owned();
    success(
        &app,
        Request::ApplyRetention {
            token: token.clone(),
            confirmation: "DELETE".into(),
        },
    )?;
    assert_eq!(
        finish(
            &app,
            Request::ApplyRetention {
                token,
                confirmation: "DELETE".into()
            }
        )?
        .state,
        "failed"
    );
    let after = success(
        &app,
        Request::Snapshots {
            repository_id: repo.id.clone(),
        },
    )?
    .result
    .unwrap();
    assert_eq!(after.as_array().unwrap().len(), 1);
    success(
        &app,
        Request::Prune {
            repository_id: repo.id.clone(),
            confirmation: "PRUNE".into(),
        },
    )?;

    // A missing/locked destination never suppresses independent destinations.
    // Completion-relative attempts prevent retry bursts, including all-locked.
    let mut scheduled = app.state.lock().unwrap().config.clone();
    scheduled.repositories.push(Repository {
        id: "locked".into(),
        label: "Locked fixture".into(),
        location: dir.path().join("locked-repo").to_str().unwrap().into(),
    });
    scheduled.profiles[0].repository_ids.push("locked".into());
    scheduled.profiles[0].schedule_minutes = 1;
    app.request(Request::SaveConfig { config: scheduled })?;
    app.state.lock().unwrap().attempts.remove(&profile.id);
    app.tick();
    let job = wait(&app)?;
    assert_eq!(job.state, "partial");
    assert_eq!(job.destinations[0].state, "success");
    assert_eq!(job.destinations[1].state, "failed");
    let count = app.state.lock().unwrap().history.len();
    app.tick();
    assert!(!app.is_busy());
    assert_eq!(app.state.lock().unwrap().history.len(), count);
    store.state.lock().unwrap().read_error = Some(credentials::LOCKED);
    app.state.lock().unwrap().attempts.remove(&profile.id);
    app.tick();
    let job = wait(&app)?;
    assert_eq!(job.state, "failed");
    assert!(
        job.destinations
            .iter()
            .all(|d| d.message == credentials::LOCKED)
    );
    app.tick();
    assert!(!app.is_busy());
    app.shutdown();
    let persisted = fs::read_to_string(app.directory.join("state.json"))?;
    assert!(!persisted.contains("second fixture"));
    assert!(!persisted.contains("\"result\":["));
    let record: Value = serde_json::from_slice(&original)?;
    let password = record["password"].as_str().unwrap();
    assert!(!persisted.contains(password));
    assert!(!fs::read_to_string(app.directory.join("config.json"))?.contains(password));
    Ok(())
}

#[cfg(unix)]
#[test]
fn cancellation_reaps_process_groups_and_bounds_output() -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let task = thread::spawn(move || {
        let mut c = restic::command("/bin/sh");
        c.args([
            "-c",
            "printf '%s\\n' '{\"message_type\":\"status\",\"percent_done\":0.1}'; sleep 60 & wait",
        ]);
        restic::run_backup(c, &signal, &mut |_| {})
    });
    thread::sleep(Duration::from_millis(100));
    cancel.store(true, Ordering::Release);
    let start = Instant::now();
    assert!(task.join().unwrap().is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    let mut c = restic::command("/bin/sh");
    c.args(["-c", "yes fixture"]);
    assert!(
        restic::run(c, None, &AtomicBool::new(false), None)
            .err()
            .unwrap()
            .to_string()
            .contains("8 MiB")
    );
    // Stream over 8 MiB through bounded progress; retain only the summary.
    let mut c = restic::command("/bin/sh");
    c.args(["-c", r#"i=0; while [ "$i" -lt 120000 ]; do printf '%s\n' '{"message_type":"status","percent_done":0.375,"bytes_done":3,"total_bytes":8,"files_done":1,"total_files":2}'; i=$((i + 1)); done; printf '%s\n' '{"message_type":"summary","snapshot_id":"fixture","total_bytes_processed":8,"total_files_processed":2}'"#]);
    let mut saw_status = false;
    let out = restic::run_backup(c, &AtomicBool::new(false), &mut |p| {
        if p.percent == Some(37.5) {
            saw_status = true;
            assert_eq!(p.bytes_done, 3);
        }
    })?;
    assert!(saw_status);
    assert_eq!(out.code, 0);
    assert!(out.bytes.len() < 1024);
    Ok(())
}

#[test]
fn config_rejects_credentials_location_changes_and_stale_revisions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &Config::default(),
    )?;
    let mut c = Config {
        repositories: vec![repository(dir.path())],
        ..Config::default()
    };
    c.repositories[0].location = "rest:https://user:secret@example.invalid/repo".into();
    assert!(
        app.request(Request::SaveConfig { config: c.clone() })
            .is_err()
    );
    c.repositories[0] = repository(dir.path());
    app.request(Request::SaveConfig { config: c.clone() })?;
    assert!(app.request(Request::SaveConfig { config: c }).is_err());
    let mut changed = app.state.lock().unwrap().config.clone();
    changed.repositories[0].location = dir.path().join("other").to_str().unwrap().into();
    assert!(
        app.request(Request::SaveConfig { config: changed })
            .is_err()
    );
    assert!(
        serde_json::from_value::<Request>(json!({"op":"unlock","repository_id":"fixture"}))
            .is_err()
    );
    assert!(serde_json::from_value::<Request>(json!({"op":"lock"})).is_err());
    assert!(config::snapshot("--latest").is_err());
    assert!(restic_path("/a/../b").is_err());
    assert_eq!(literal_pattern("/a[*]?.txt"), "/a\\[\\*\\]\\?.txt");
    let status = app.request(Request::Status)?;
    assert_eq!(status.as_object().unwrap().len(), 5);
    assert_eq!(status["config"].as_object().unwrap().len(), 4);
    assert_eq!(store.state.lock().unwrap().reads, 0);
    let repo = repository(dir.path());
    fs::create_dir(&repo.location)?;
    fs::write(Path::new(&repo.location).join("old-backup"), b"untouched")?;
    assert!(
        finish(&app, initialize(&repo))?
            .message
            .contains("new or empty")
    );
    assert_eq!(store.state.lock().unwrap().writes, 0);
    assert_eq!(
        fs::read(Path::new(&repo.location).join("old-backup"))?,
        b"untouched"
    );
    app.shutdown();
    assert!(app.request(initialize(&repo)).is_err());
    assert!(config::read(dir.path()).is_err());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let shared = dir.path().join("shared");
        fs::create_dir(&shared)?;
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o777))?;
        assert!(private_restore_directory(&shared.join("out")).is_err());
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777))?;
        assert!(private_restore_directory(&shared.join("out")).is_ok());
    }
    Ok(())
}

#[test]
fn keychain_errors_do_not_create_or_replace_credentials() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(FakeStore::default());
    let repo = repository(dir.path());
    let c = Config {
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    assert!(finish(&app, check(&repo))?.message.contains("missing"));
    for error in [
        credentials::LOCKED,
        credentials::UNAVAILABLE,
        "Duplicate system keychain entries",
    ] {
        store.state.lock().unwrap().read_error = Some(error);
        let reads = store.state.lock().unwrap().reads;
        app.request(Request::Status)?;
        assert_eq!(store.state.lock().unwrap().reads, reads);
        assert_eq!(finish(&app, initialize(&repo))?.message, error);
        assert_eq!(store.state.lock().unwrap().writes, 0);
    }
    store.state.lock().unwrap().read_error = None;
    let original = credentials::Record::generate(&repo).bytes()?.to_vec();
    for malformed in [
        b"not JSON".to_vec(),
        serde_json::to_vec(
            &json!({"version":2,"location":repo.location,"password":"a".repeat(64)}),
        )?,
        serde_json::to_vec(&json!({"version":1,"location":"elsewhere","password":"a".repeat(64)}))?,
    ] {
        store
            .state
            .lock()
            .unwrap()
            .entries
            .insert(repo.id.clone(), malformed.clone());
        assert_eq!(finish(&app, initialize(&repo))?.state, "failed");
        assert_eq!(store.state.lock().unwrap().writes, 0);
        assert_eq!(store.state.lock().unwrap().entries[&repo.id], malformed);
    }
    store.state.lock().unwrap().entries.clear();
    store.state.lock().unwrap().write_mode = 1;
    assert_eq!(finish(&app, initialize(&repo))?.state, "failed");
    assert!(!Path::new(&repo.location).exists());
    assert!(store.state.lock().unwrap().entries.is_empty());
    // Removing config does not erase an entry. Re-adding an ID still checks
    // its stored locator before any restic invocation.
    store
        .state
        .lock()
        .unwrap()
        .entries
        .insert(repo.id.clone(), original.clone());
    let mut c = c;
    c.repositories.clear();
    app.request(Request::SaveConfig { config: c })?;
    let mut c = app.state.lock().unwrap().config.clone();
    let mut moved = repo.clone();
    moved.location = dir.path().join("moved").to_str().unwrap().into();
    c.repositories.push(moved.clone());
    app.request(Request::SaveConfig { config: c })?;
    assert!(
        finish(&app, check(&moved))?
            .message
            .contains("another location")
    );
    assert_eq!(store.state.lock().unwrap().entries[&repo.id], original);
    app.shutdown();
    Ok(())
}

#[cfg(unix)]
#[test]
fn initialization_reuses_committed_credentials_and_only_initializes_after_exit_ten() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let executable = dir.path().join("fixture-restic");
    let code = dir.path().join("probe-code");
    let response = dir.path().join("probe-response");
    let events = dir.path().join("events");
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
if [ "$1" = version ]; then printf 'restic 0.18.1\n'; exit 0; fi
for arg in "$@"; do
  case "$arg" in
    *"$RESTIC_PASSWORD"*) exit 98 ;;
  esac
done
while [ "$#" -gt 0 ]; do
 case "$1" in
 cat) printf 'cat\n' >> '{}'; code=$(cat '{}'); if [ "$code" -eq 0 ]; then cat '{}'; fi; exit "$code" ;;
 init) printf 'init\n' >> '{}'; exit 0 ;;
 check) printf '%s' "$RESTIC_PASSWORD"; exit 0 ;;
 esac
 shift
done
exit 99
"#,
            events.display(),
            code.display(),
            response.display(),
            events.display()
        ),
    )?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    fs::write(&code, "10")?;
    fs::write(&events, "")?;
    let repo = Repository {
        location: "sftp:fixture.invalid:/never-accessed".into(),
        ..repository(dir.path())
    };
    let c = Config {
        restic_path: executable.to_str().unwrap().into(),
        repositories: vec![repo.clone()],
        ..Config::default()
    };
    let store = Arc::new(FakeStore::default());
    store.state.lock().unwrap().write_mode = 2; // commit, then report failure
    let app = open(
        dir.path().join("app"),
        store.clone(),
        dir.path().join("locks"),
        &c,
    )?;
    assert_eq!(finish(&app, initialize(&repo))?.state, "failed");
    assert_eq!(fs::read_to_string(&events)?, "");
    let original = store.state.lock().unwrap().entries[&repo.id].clone();
    store.state.lock().unwrap().write_mode = 0;
    for (exit, identity) in [
        (0, true),
        (10, true),
        (11, true),
        (12, true),
        (1, true),
        (0, false),
    ] {
        fs::write(&code, exit.to_string())?;
        fs::write(
            &response,
            serde_json::to_vec(
                &json!({"id":if identity { "a".repeat(64) } else { "invalid".into() }}),
            )?,
        )?;
        fs::write(&events, "")?;
        let job = finish(&app, initialize(&repo))?;
        assert_eq!(
            job.state,
            if exit == 10 || (exit == 0 && identity) {
                "success"
            } else {
                "failed"
            }
        );
        assert_eq!(
            fs::read_to_string(&events)?,
            if exit == 10 { "cat\ninit\n" } else { "cat\n" }
        );
        assert_eq!(store.state.lock().unwrap().entries[&repo.id], original);
        assert_eq!(store.state.lock().unwrap().writes, 1);
    }
    // A subprocess that prints the environment password cannot forward it to
    // IPC results, job errors, or persisted state.
    assert!(
        finish(&app, check(&repo))?
            .message
            .contains("response withheld")
    );
    let password = credentials::required(store.as_ref(), &repo)?
        .password
        .clone();
    assert!(
        !app.request(Request::Status)?
            .to_string()
            .contains(&password)
    );
    for entry in fs::read_dir(&app.directory)? {
        let path = entry?.path();
        if path.is_file() {
            assert!(!fs::read_to_string(path)?.contains(&password));
        }
    }
    app.shutdown();
    Ok(())
}
