use super::*;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

#[derive(Default)]
struct MemoryStore(Mutex<Option<Vec<u8>>>);
impl Store for MemoryStore {
    fn get(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.0.lock().unwrap().clone().map(Zeroizing::new))
    }
    fn create(&self, _: &str, bytes: &[u8]) -> Result<()> {
        *self.0.lock().unwrap() = Some(bytes.to_vec());
        Ok(())
    }
}
fn fixture(path: &Path, executable: &str) -> Result<(Arc<App>, Repository)> {
    let repo = Repository {
        id: "console".into(),
        label: "Console fixture".into(),
        location: path.join("repo").to_string_lossy().into_owned(),
    };
    let source = path.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("fixture-file"), b"console smoke test")?;
    let c = Config {
        restic_path: executable.into(),
        repositories: vec![repo.clone()],
        profiles: vec![Profile {
            id: "console-profile".into(),
            label: "Console profile".into(),
            sources: vec![source.to_string_lossy().into_owned()],
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
    let directory = path.join("app");
    fs::create_dir(&directory)?;
    fs::write(directory.join("config.json"), serde_json::to_vec(&c)?)?;
    let store = Arc::new(MemoryStore::default());
    store.create(&repo.id, &credentials::Record::generate(&repo).bytes()?)?;
    Ok((App::open_inner(directory, store, path.join("locks"))?, repo))
}
fn finish(app: &Arc<App>, request: Request) -> Result<Job> {
    app.request(request)?;
    let started = Instant::now();
    while app.is_busy() {
        ensure!(
            started.elapsed() < Duration::from_secs(30),
            "fixture timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
    Ok(app.state.lock().unwrap().status.history[0].clone())
}

#[test]
fn real_restic_console_retains_file_activity_only_for_current_session() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (app, repo) = fixture(dir.path(), "/usr/bin/restic")?;
    assert_eq!(
        finish(
            &app,
            Request::Initialize {
                repository_id: repo.id.clone()
            }
        )?
        .state,
        "success"
    );
    let mut notification_cursor = None;
    assert!(app.notification_states(&mut notification_cursor).is_empty());
    let backup = finish(
        &app,
        Request::Backup {
            profile_id: "console-profile".into(),
        },
    )?;
    assert_eq!(backup.state, "success");
    assert!(
        backup.output.iter().any(|s| s.contains("fixture-file")),
        "{:?}",
        backup.output
    );
    assert!(backup.output.iter().any(|s| s.contains("Summary:")));
    let query = finish(
        &app,
        Request::Snapshots {
            repository_id: repo.id,
        },
    )?;
    assert_eq!(query.state, "success");
    assert_eq!(
        app.notification_states(&mut notification_cursor),
        ["success"]
    );
    assert_eq!(notification_cursor.as_ref(), Some(&query.id));
    assert!(app.notification_states(&mut notification_cursor).is_empty());
    assert!(
        !query
            .output
            .iter()
            .any(|s| s.contains("fixture-file") || s.contains("snapshot_id"))
    );
    let status = app.request(Request::Status)?;
    assert!(status["history"].as_array().unwrap().iter().any(|job| {
        job["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("fixture-file"))
    }));
    let persisted = fs::read_to_string(app.directory.join("state.json"))?;
    assert!(!persisted.contains("fixture-file"));
    assert!(!persisted.contains("\"output\"") && !persisted.contains("\"output_truncated\""));
    let saved: Persisted = serde_json::from_str(&persisted)?;
    assert!(
        saved
            .history
            .iter()
            .all(|j| j.output.is_empty() && !j.output_truncated)
    );
    app.shutdown();
    let directory = app.directory.clone();
    let locks = app.credential_locks.clone();
    let store = app.credentials.clone();
    drop(app);
    let reopened = App::open_inner(directory, store, locks)?;
    assert!(
        reopened
            .state
            .lock()
            .unwrap()
            .status
            .history
            .iter()
            .all(|j| j.output.is_empty())
    );
    reopened.shutdown();
    Ok(())
}

#[cfg(unix)]
fn executable(path: &Path, script: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, script)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_and_cancelled_jobs_keep_sanitized_live_output_without_persisting_it() -> Result<()> {
    for cancelled in [false, true] {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("fixture-restic");
        let ack = dir.path().join("ack");
        executable(
            &script,
            &format!(
                r#"#!/usr/bin/python3
import os,sys,time,json
if 'version' in sys.argv:
    print('restic 0.18.1')
    sys.exit(0)
assert os.environ['RESTIC_PROGRESS_FPS'] == '1'
password=os.environ['RESTIC_PASSWORD']
print('live-file stdout '+password,flush=True)
sys.stderr.write('live-file stderr '+password+'\n')
sys.stderr.flush()
escaped=''.join('\\u%04x'%ord(c) for c in password)
print('\x1b[31m{{"message_type":"verbose_status","action":"new","item":"'+escaped+'"}}\x1b[0m',flush=True)
print('{{"item":"'+escaped+'"',flush=True)
print('x'*1018+password,flush=True)
end=time.monotonic()+5
while not os.path.exists({ack:?}):
    if time.monotonic()>end: sys.exit(8)
    time.sleep(.01)
sys.exit(2)
"#,
                ack = ack.to_str().unwrap()
            ),
        )?;
        let (app, repo) = fixture(dir.path(), script.to_str().unwrap())?;
        app.request(Request::Check {
            repository_id: repo.id,
        })?;
        let started = Instant::now();
        loop {
            let status = app.request(Request::Status)?;
            let lines = status["active"]["output"]
                .as_array()
                .context("job exited before live output")?;
            if lines
                .iter()
                .any(|line| line.as_str().unwrap().contains("live-file stderr"))
            {
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(3),
                "no live console data"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let active = fs::read_to_string(app.directory.join("state.json"))?;
        assert!(
            !active.contains("\"output\"")
                && !active.contains("\"output_truncated\"")
                && !active.contains("live-file")
        );
        if cancelled {
            app.request(Request::Cancel)?;
        } else {
            fs::write(&ack, b"continue")?;
        }
        while app.is_busy() {
            ensure!(
                started.elapsed() < Duration::from_secs(5),
                "job did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let job = app.state.lock().unwrap().status.history[0].clone();
        assert_eq!(job.state, if cancelled { "cancelled" } else { "failed" });
        assert!(
            job.output
                .iter()
                .any(|s| s.contains("live-file stdout [redacted]"))
        );
        assert!(
            job.output
                .iter()
                .any(|s| s.contains("live-file stderr [redacted]"))
        );
        assert!(job.output.iter().any(|s| s == "new: [redacted]"));
        assert!(job.output.iter().all(|s| !s.contains("\\u")));
        assert!(job.output_truncated);
        let record = credentials::required(
            app.credentials.as_ref(),
            &app.state.lock().unwrap().status.config.repositories[0],
        )?;
        assert!(
            !app.request(Request::Status)?
                .to_string()
                .contains(&record.password)
        );
        assert!(!app.request(Request::Status)?.to_string().contains("\\\\u"));
        let persisted = fs::read_to_string(app.directory.join("state.json"))?;
        assert!(
            !persisted.contains("\"output\"")
                && !persisted.contains("\"output_truncated\"")
                && !persisted.contains("live-file")
        );
        app.shutdown();
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn real_restic_emits_status_before_exit_when_stdout_is_not_a_terminal() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let wrapper = dir.path().join("slow-restic");
    // Throttle this disposable local repository, not any external service. This
    // makes a periodic status observable even on a fast machine.
    executable(
        &wrapper,
        r#"#!/usr/bin/python3
import os,sys
args=sys.argv[1:]
if 'backup' in args:
    assert '--verbose=2' in args
    args.insert(args.index('backup')+1,'--limit-upload=128')
os.execv('/usr/bin/restic',['restic']+args)
"#,
    )?;
    let (app, repo) = fixture(dir.path(), wrapper.to_str().unwrap())?;
    let mut seed = 0x12345678u32;
    let bytes: Vec<u8> = (0..524288)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    fs::write(dir.path().join("source/slow-file"), bytes)?;
    assert_eq!(
        finish(
            &app,
            Request::Initialize {
                repository_id: repo.id
            }
        )?
        .state,
        "success"
    );
    app.request(Request::Backup {
        profile_id: "console-profile".into(),
    })?;
    let started = Instant::now();
    let mut live_status = false;
    while app.is_busy() {
        let status = app.request(Request::Status)?;
        live_status |= status["active"]["output"].as_array().is_some_and(|lines| {
            lines
                .iter()
                .any(|line| line.as_str().unwrap().starts_with("Status:"))
        });
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "backup timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.state.lock().unwrap().status.history[0].state, "success");
    assert!(
        live_status,
        "no periodic non-TTY status arrived before exit"
    );
    app.shutdown();
    Ok(())
}
