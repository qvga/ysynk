use crate::{
    config,
    credentials::{self, NativeStore, Store},
    models::*,
    restic,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    history: Vec<Job>,
    attempts: BTreeMap<String, u64>,
    active: Option<Job>,
}
struct Preview {
    expires: u64,
    revision: u64,
    repository: Repository,
    profile_id: String,
    repository_identity: String,
    remove: Vec<Value>,
    survivors: Vec<String>,
}
struct State {
    password_export: bool,
    config: Config,
    active: Option<Job>,
    history: Vec<Job>,
    attempts: BTreeMap<String, u64>,
    previews: HashMap<String, Preview>,
    version: Option<String>,
    setup_error: Option<String>,
}
pub struct App {
    directory: PathBuf,
    credentials: Arc<dyn Store>,
    credential_locks: PathBuf,
    state: Mutex<State>,
    export_finished: Condvar,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    cancel: AtomicBool,
    closing: AtomicBool,
    _lock: File,
}
struct PasswordExport<'a>(&'a App);
impl Drop for PasswordExport<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.password_export = false;
        self.0.export_finished.notify_all();
    }
}

impl App {
    pub fn open(directory: PathBuf) -> Result<Arc<Self>> {
        let locks = dirs::data_local_dir()
            .context("could not locate per-user local application data for credential locks")?
            .join("io.ysynk.restic")
            .join("keychain-locks");
        Self::open_inner(directory, Arc::new(NativeStore), locks)
    }
    fn open_inner(
        directory: PathBuf,
        credentials: Arc<dyn Store>,
        credential_locks: PathBuf,
    ) -> Result<Arc<Self>> {
        fs::create_dir_all(&directory)?;
        ensure!(
            fs::symlink_metadata(&directory)?.is_dir(),
            "data directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("app.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("another app instance owns this data directory")?;
        let cache = directory.join("cache");
        fs::create_dir_all(&cache)?;
        ensure!(
            fs::symlink_metadata(&cache)?.is_dir(),
            "cache must be a private directory, not a symlink"
        );
        let config_path = directory.join("config.json");
        let config: Config = if config_path.exists() {
            serde_json::from_slice(&config::read(&config_path)?)?
        } else {
            Config::default()
        };
        config::validate_shape(&config)?;
        config::validate_overlap(&config, &directory)?;
        let saved: Persisted = if directory.join("state.json").exists() {
            serde_json::from_slice(&config::read(&directory.join("state.json"))?)?
        } else {
            Persisted::default()
        };
        let mut history = saved.history;
        if let Some(mut interrupted) = saved.active {
            interrupted.state = "interrupted".into();
            interrupted.message =
                "App stopped before this job completed; inspect repository state before retrying"
                    .into();
            interrupted.finished_at = Some(now());
            interrupted.result = None;
            history.insert(0, interrupted);
        }
        history.truncate(100);
        let (version, setup_error) = match restic::version(&config.restic_path) {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let app = Arc::new(Self {
            directory,
            credentials,
            credential_locks,
            state: Mutex::new(State {
                password_export: false,
                config,
                active: None,
                history,
                attempts: saved.attempts,
                previews: HashMap::new(),
                version,
                setup_error,
            }),
            export_finished: Condvar::new(),
            workers: Mutex::new(vec![]),
            cancel: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            _lock: lock,
        });
        app.persist(&app.state.lock().unwrap())?;
        Ok(app)
    }
    fn persist(&self, state: &State) -> Result<()> {
        let mut history = state.history.clone();
        for job in &mut history {
            job.result = None;
        }
        let saved = Persisted {
            history,
            attempts: state.attempts.clone(),
            active: state.active.clone(),
        };
        config::write(
            &self.directory.join("state.json"),
            &serde_json::to_vec(&saved)?,
            true,
        )
    }
    fn expire(state: &mut State) {
        state.previews.retain(|_, p| p.expires > now());
    }
    fn status(&self, state: &mut State) -> Result<Value> {
        Self::expire(state);
        Ok(serde_json::to_value(Status {
            config: state.config.clone(),
            active: state.active.clone(),
            history: state.history.clone(),
            restic_version: state.version.clone(),
            setup_error: state.setup_error.clone(),
        })?)
    }
    pub fn is_busy(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.active.is_some() || state.password_export
    }
    /// Validate native password-export admission without accessing credentials.
    /// The desktop calls this before opening its trusted native Save dialog.
    pub fn validate_repository_password_export(
        &self,
        repository_id: &str,
        revision: u64,
    ) -> Result<()> {
        let state = self.state.lock().unwrap();
        self.export_selection(&state, repository_id, revision)?;
        Ok(())
    }
    fn export_selection(
        &self,
        state: &State,
        repository_id: &str,
        revision: u64,
    ) -> Result<Repository> {
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "App is shutting down; password export was not completed"
        );
        ensure!(
            state.config.revision == revision,
            "Configuration changed; reload before exporting a repository password"
        );
        Self::repository(&state.config, repository_id).map_err(|_| {
            anyhow::anyhow!(
                "Repository is no longer configured; reload before exporting its password"
            )
        })
    }
    /// Write a recovery password to a new file selected by the native desktop.
    /// This is deliberately not a Request and never returns the secret or path.
    pub fn export_repository_password(
        &self,
        repository_id: &str,
        revision: u64,
        target: PathBuf,
    ) -> Result<()> {
        use std::io::Write;
        let (repo, configuration, _reservation) = {
            let mut state = self.state.lock().unwrap();
            let repo = self.export_selection(&state, repository_id, revision)?;
            ensure!(
                state.active.is_none() && !state.password_export,
                "Another operation is running; wait before exporting a repository password"
            );
            state.password_export = true;
            (repo, state.config.clone(), PasswordExport(self))
        };
        let target = password_export_target(&configuration, &self.directory, &target)?;
        // Native calls may block. Never hold the state mutex across this read.
        // required() is read-only and checks the stored version and locator.
        let record = credentials::required(self.credentials.as_ref(), &repo)?;
        {
            let state = self.state.lock().unwrap();
            let current = self.export_selection(&state, repository_id, revision)?;
            ensure!(
                current.location == repo.location,
                "Repository location changed; password export was not completed"
            );
        }
        let parent = target
            .parent()
            .context("Choose a new password file in an existing private folder")?;
        // NamedTempFile creates a new 0600 file on Unix. Staging and syncing
        // before publication means any write failure cleans up only the owned
        // temporary file, never an existing selected target or symlink.
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| {
            anyhow::anyhow!(
                "Could not create a private password file; select a writable private folder"
            )
        })?;
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(record.password.len() + 1));
        bytes.extend_from_slice(record.password.as_bytes());
        bytes.push(b'\n');
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| {
                anyhow::anyhow!(
                    "Could not write and sync the password file; no export was completed"
                )
            })?;
        // The reservation prevents config saves and job admission without
        // blocking status. Recheck directory links outside the state mutex.
        ensure!(
            password_export_target(&configuration, &self.directory, &target)? == target,
            "Password export folder changed; select the destination again"
        );
        {
            let state = self.state.lock().unwrap();
            let current = self.export_selection(&state, repository_id, revision)?;
            ensure!(
                current.location == repo.location,
                "Repository location changed; password export was not completed"
            );
        }
        file.persist_noclobber(&target).map_err(|_| {
            anyhow::anyhow!(
                "Could not save a new password file; existing files are never overwritten"
            )
        })?;
        // Nothing fallible after publication: never attempt a path-based
        // rollback that could remove a file another process replaced.
        Ok(())
    }
    pub fn request(self: &Arc<Self>, request: Request) -> Result<Value> {
        if matches!(request, Request::Cancel) {
            let _state = self.state.lock().unwrap();
            self.cancel.store(true, Ordering::Release);
            return Ok(Value::Null);
        }
        let mut state = self.state.lock().unwrap();
        Self::expire(&mut state);
        if matches!(request, Request::Status) {
            return self.status(&mut state);
        }
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "app is shutting down"
        );
        ensure!(
            !state.password_export,
            "A repository password export is running; wait for it to finish"
        );
        ensure!(
            state.active.is_none(),
            "another job is running; cancel it and wait for completion"
        );
        match request {
            Request::SaveConfig { mut config } => {
                ensure!(
                    config.revision == state.config.revision,
                    "configuration changed; reload before saving"
                );
                config::validate(&config)?;
                config::validate_overlap(&config, &self.directory)?;
                for old in &state.config.repositories {
                    if let Some(new) = config.repositories.iter().find(|r| r.id == old.id) {
                        ensure!(
                            old.location == new.location,
                            "Repository location cannot change for an existing ID; use a new repository ID"
                        );
                    }
                }
                // Save/status never access the keychain. Location binding is
                // checked when each command fetches its credential. Removing
                // configuration deliberately leaves native entries untouched.
                let changed_path = config.restic_path != state.config.restic_path;
                config.revision = config
                    .revision
                    .checked_add(1)
                    .context("configuration revision overflow")?;
                config::write(
                    &self.directory.join("config.json"),
                    &serde_json::to_vec(&config)?,
                    true,
                )?;
                state.config = config;
                state.previews.clear();
                if changed_path {
                    state.version = None;
                    state.setup_error = Some(
                        "Restic executable changed; restart the app to validate its version".into(),
                    );
                }
                self.status(&mut state)
            }
            request => {
                ensure!(
                    state.setup_error.is_none(),
                    "{}",
                    state.setup_error.as_deref().unwrap_or("restic unavailable")
                );
                let (kind, profile_id, repository_id) = request_scope(&request);
                if let Some(id) = &repository_id {
                    ensure!(
                        state.config.repositories.iter().any(|r| &r.id == id),
                        "unknown repository"
                    );
                }
                if let Some(id) = &profile_id {
                    ensure!(
                        state.config.profiles.iter().any(|p| &p.id == id),
                        "unknown profile"
                    );
                }
                let job_id = id();
                state.active = Some(Job {
                    id: job_id.clone(),
                    kind: kind.into(),
                    label: kind.replace('_', " "),
                    state: "running".into(),
                    started_at: now(),
                    finished_at: None,
                    profile_id: profile_id.clone(),
                    repository_id,
                    message: "Running".into(),
                    progress: None,
                    destinations: vec![],
                    result: None,
                });
                if kind == "backup"
                    && let Some(id) = profile_id
                {
                    state.attempts.insert(id, now());
                }
                if let Err(e) = self.persist(&state) {
                    state.active = None;
                    return Err(e);
                }
                self.cancel.store(false, Ordering::Release);
                let config = state.config.clone();
                let app = Arc::clone(self);
                let mut workers = self.workers.lock().unwrap();
                for handle in workers.extract_if(.., |h| h.is_finished()) {
                    let _ = handle.join();
                }
                workers.push(thread::spawn(move || {
                    let outcome = app.perform(request, &config);
                    app.complete(outcome);
                }));
                Ok(json!({"job_id": job_id}))
            }
        }
    }
    fn complete(&self, outcome: Result<Value>) {
        let mut state = self.state.lock().unwrap();
        let mut job = state.active.take().unwrap();
        job.finished_at = Some(now());
        if self.cancel.load(Ordering::Acquire) {
            job.state = "cancelled".into();
            job.message = "Operation cancelled; incomplete restore output is preserved".into();
        } else {
            match outcome {
                Ok(value) => {
                    if job.state == "running" {
                        job.state = "success".into();
                        job.message = "Completed".into();
                    }
                    job.result = Some(value);
                }
                Err(error) => {
                    job.state = "failed".into();
                    job.message = error.to_string();
                }
            }
        }
        if job.kind == "backup"
            && let Some(id) = &job.profile_id
        {
            state.attempts.insert(id.clone(), now());
        }
        state.history.insert(0, job);
        state.history.truncate(100);
        let mut budget = 0;
        for job in &mut state.history {
            if let Some(result) = &job.result {
                budget += serde_json::to_vec(result).map_or(config::LIMIT + 1, |b| b.len());
                if budget > config::LIMIT {
                    job.result = None;
                }
            }
        }
        if self.persist(&state).is_err() {
            state.setup_error = Some("Could not save job history and schedule attempts; fix data directory access before continuing".into());
        }
    }
    pub fn tick(self: &Arc<Self>) {
        let due = {
            let mut state = self.state.lock().unwrap();
            Self::expire(&mut state);
            if state.active.is_some()
                || state.password_export
                || self.closing.load(Ordering::Acquire)
                || state.setup_error.is_some()
            {
                return;
            }
            state
                .config
                .profiles
                .iter()
                .find(|p| {
                    p.schedule_minutes > 0
                        && now().saturating_sub(*state.attempts.get(&p.id).unwrap_or(&0))
                            >= p.schedule_minutes * 60
                })
                .map(|p| p.id.clone())
        };
        if let Some(profile_id) = due {
            let _ = self.request(Request::Backup { profile_id });
        }
    }
    pub fn shutdown(&self) {
        // Admission holds state then workers; keep the same lock order, releasing state before joins.
        let state = self.state.lock().unwrap();
        self.closing.store(true, Ordering::Release);
        self.cancel.store(true, Ordering::Release);
        let handles = std::mem::take(&mut *self.workers.lock().unwrap());
        drop(state);
        for handle in handles {
            let _ = handle.join();
        }
        let mut state = self.state.lock().unwrap();
        while state.password_export {
            state = self.export_finished.wait(state).unwrap();
        }
    }
    fn repository(c: &Config, id: &str) -> Result<Repository> {
        c.repositories
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .context("unknown repository")
    }
    fn profile(c: &Config, id: &str) -> Result<Profile> {
        c.profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .context("unknown profile")
    }
    fn run(&self, c: &Config, repo: &Repository, args: &[String]) -> Result<restic::Output> {
        let record = credentials::required(self.credentials.as_ref(), repo)?;
        let mut command = restic::command(&c.restic_path);
        command
            .env("RESTIC_REPOSITORY", &repo.location)
            .env("RESTIC_PASSWORD", &record.password);
        command
            .arg("--json")
            .arg("--cache-dir")
            .arg(self.directory.join("cache"))
            .args(args);
        let output = if args.first().is_some_and(|arg| arg == "backup") {
            self.state.lock().unwrap().active.as_mut().unwrap().progress = None;
            restic::run_backup(command, &self.cancel, &mut |progress| {
                if let Some(job) = self.state.lock().unwrap().active.as_mut() {
                    job.progress = Some(progress);
                }
            })
        } else {
            restic::run(command, None, &self.cancel, None)
        }?;
        ensure!(
            !output
                .bytes
                .windows(record.password.len())
                .any(|part| part == record.password.as_bytes()),
            "Restic output contained a credential; response withheld"
        );
        Ok(output)
    }
    fn query(&self, c: &Config, repo: &Repository, args: &[String]) -> Result<Value> {
        let output = self.run(c, repo, args)?;
        restic::successful(&output)?;
        serde_json::from_slice(&output.bytes)
            .map_err(|_| anyhow::anyhow!("unexpected restic JSON response"))
    }
    fn perform(&self, request: Request, c: &Config) -> Result<Value> {
        match request {
            Request::Initialize { repository_id } => {
                let repo = Self::repository(c, &repository_id)?;
                // The native namespace is shared by all data directories. Hold
                // this per-user, cross-process lock through lookup, create and
                // init so another app instance cannot generate a replacement.
                let _guard = credentials::creation_lock(&self.credential_locks, &repo.id)?;
                if credentials::read(self.credentials.as_ref(), &repo)?.is_none() {
                    self.check_local_initialization(&repo)?;
                    let record = credentials::Record::generate(&repo);
                    self.credentials.create(&repo.id, &record.bytes()?)?;
                    // If a write reports failure after committing, this job
                    // stops. The next retry reads that entry, never overwrites.
                }
                // Every backend, including the first attempt, must authenticate
                // the preserved keychain entry. Only exit 10 permits init.
                let probe = self.run(c, &repo, &strings(&["cat", "config"]))?;
                match probe.code {
                    0 => {
                        let value: Value = serde_json::from_slice(&probe.bytes).map_err(|_| {
                            anyhow::anyhow!("unexpected repository configuration response")
                        })?;
                        let identity = repository_config_id(&value)?;
                        return Ok(json!({"reconnected":true,"repository_identity":identity}));
                    }
                    10 => self.check_local_initialization(&repo)?,
                    _ => restic::successful(&probe)?,
                }
                let output = self.run(c, &repo, &strings(&["init"]))?;
                restic::successful(&output)?;
                Ok(Value::Null)
            }
            Request::Backup { profile_id } => self.backup(c, &Self::profile(c, &profile_id)?),
            Request::Snapshots { repository_id } => self.query(
                c,
                &Self::repository(c, &repository_id)?,
                &strings(&["snapshots"]),
            ),
            Request::Stats { repository_id } => self.query(
                c,
                &Self::repository(c, &repository_id)?,
                &strings(&["stats", "--mode", "raw-data"]),
            ),
            Request::Check { repository_id } => {
                let output = self.run(
                    c,
                    &Self::repository(c, &repository_id)?,
                    &strings(&["check"]),
                )?;
                restic::successful(&output)?;
                Ok(Value::Null)
            }
            Request::Prune {
                repository_id,
                confirmation,
            } => {
                ensure!(
                    confirmation == "PRUNE",
                    "type PRUNE to confirm repository-wide maintenance"
                );
                let output = self.run(
                    c,
                    &Self::repository(c, &repository_id)?,
                    &strings(&["prune"]),
                )?;
                restic::successful(&output)?;
                Ok(Value::Null)
            }
            Request::ListFiles {
                repository_id,
                snapshot_id,
                path,
            } => {
                config::snapshot(&snapshot_id)?;
                restic_path(&path)?;
                let output = self.run(
                    c,
                    &Self::repository(c, &repository_id)?,
                    &strings(&["ls", &snapshot_id, "--", &path]),
                )?;
                restic::successful(&output)?;
                let mut nodes = vec![];
                for line in output
                    .bytes
                    .split(|b| *b == b'\n')
                    .filter(|s| !s.is_empty())
                {
                    let node: Value = serde_json::from_slice(line)
                        .map_err(|_| anyhow::anyhow!("unexpected restic file listing"))?;
                    if node["struct_type"] == "node" {
                        let p = node["path"].as_str().context("node missing path")?;
                        if Path::new(p).parent() == Some(Path::new(&path)) {
                            nodes.push(node);
                        }
                    }
                }
                Ok(json!(nodes))
            }
            Request::Restore {
                repository_id,
                snapshot_id,
                paths,
                target,
            } => self.restore(
                c,
                &Self::repository(c, &repository_id)?,
                &snapshot_id,
                &paths,
                &target,
            ),
            Request::RetentionPreview {
                repository_id,
                profile_id,
            } => self.preview(
                c,
                Self::repository(c, &repository_id)?,
                Self::profile(c, &profile_id)?,
            ),
            Request::ApplyRetention {
                token,
                confirmation,
            } => self.apply_retention(c, &token, &confirmation),
            _ => bail!("request cannot run as a job"),
        }
    }
    fn backup(&self, c: &Config, profile: &Profile) -> Result<Value> {
        config::validate_overlap(c, &self.directory)?;
        let mut args = strings(&["backup", "--tag", &format!("ysynk-profile:{}", profile.id)]);
        for exclude in &profile.excludes {
            args.push("--exclude".into());
            args.push(exclude.clone());
        }
        args.push("--".into());
        args.extend(profile.sources.clone());
        let mut success = 0;
        let mut partial = 0;
        for id in &profile.repository_ids {
            if self.cancel.load(Ordering::Acquire) {
                bail!("operation cancelled");
            }
            let repo = Self::repository(c, id)?;
            let outcome = self.run(c, &repo, &args);
            let mut snapshot_id = None;
            let (state, message) = match outcome {
                Ok(output) => {
                    if output.code == 0 {
                        success += 1;
                    } else if output.code == 3 {
                        partial += 1;
                    }
                    for line in output.bytes.split(|b| *b == b'\n') {
                        if let Ok(v) = serde_json::from_slice::<Value>(line)
                            && v["message_type"] == "summary"
                        {
                            snapshot_id = v["snapshot_id"].as_str().map(str::to_owned);
                            if output.code == 0 {
                                let bytes = v["total_bytes_processed"].as_u64().unwrap_or(0);
                                let files = v["total_files_processed"].as_u64().unwrap_or(0);
                                self.state.lock().unwrap().active.as_mut().unwrap().progress =
                                    Some(Progress {
                                        percent: Some(100.0),
                                        bytes_done: bytes,
                                        total_bytes: bytes,
                                        files_done: files,
                                        total_files: files,
                                    });
                            }
                        }
                    }
                    (
                        if output.code == 0 {
                            "success"
                        } else if output.code == 3 {
                            "partial"
                        } else {
                            "failed"
                        },
                        restic::exit_message(output.code).to_owned(),
                    )
                }
                Err(error) => ("failed", error.to_string()),
            };
            self.state
                .lock()
                .unwrap()
                .active
                .as_mut()
                .unwrap()
                .destinations
                .push(DestinationResult {
                    repository_id: id.clone(),
                    state: state.into(),
                    message,
                    snapshot_id,
                });
        }
        let mut state = self.state.lock().unwrap();
        let job = state.active.as_mut().unwrap();
        job.state = if success == profile.repository_ids.len() {
            "success"
        } else if success > 0 || partial > 0 {
            "partial"
        } else {
            "failed"
        }
        .into();
        job.message = format!(
            "{} of {} destinations completed; {} incomplete",
            success,
            profile.repository_ids.len(),
            partial
        );
        Ok(Value::Null)
    }
    fn repository_identity(&self, c: &Config, repo: &Repository) -> Result<String> {
        repository_config_id(&self.query(c, repo, &strings(&["cat", "config"]))?)
    }
    fn check_local_initialization(&self, repo: &Repository) -> Result<()> {
        if let Some(path) = config::local_repository(&repo.location)
            && path.exists()
        {
            ensure!(
                fs::symlink_metadata(path)?.is_dir(),
                "local repository path must be a directory, not a symlink"
            );
            let mut entries = fs::read_dir(path)?;
            if entries.next().transpose()?.is_some() {
                if path.join("config").is_file() {
                    bail!(
                        "local directory already contains a config file; initialization refused (existing repository import is not supported)"
                    );
                }
                bail!(
                    "local initialization requires a new or empty directory; existing files were left untouched"
                );
            }
        }
        Ok(())
    }
    fn preview(&self, c: &Config, repo: Repository, profile: Profile) -> Result<Value> {
        ensure!(
            profile.repository_ids.contains(&repo.id),
            "profile does not use this repository"
        );
        let identity = self.repository_identity(c, &repo)?;
        let policy = &profile.retention;
        ensure!(
            policy.keep_last >= 1,
            "retention must keep at least one snapshot"
        );
        let args = strings(&[
            "forget",
            "--dry-run",
            "--group-by",
            "host,paths",
            "--tag",
            &format!("ysynk-profile:{}", profile.id),
            "--keep-last",
            &policy.keep_last.to_string(),
            "--keep-daily",
            &policy.keep_daily.to_string(),
            "--keep-weekly",
            &policy.keep_weekly.to_string(),
            "--keep-monthly",
            &policy.keep_monthly.to_string(),
        ]);
        let result = self.query(c, &repo, &args)?;
        let mut remove = vec![];
        let mut survivors = vec![];
        for group in result.as_array().context("unexpected retention preview")? {
            if let Some(kept) = group["keep"].as_array() {
                for snapshot in kept {
                    survivors.push(full_snapshot_id(snapshot)?);
                }
            }
            if let Some(deleted) = group["remove"].as_array() {
                for snapshot in deleted {
                    full_snapshot_id(snapshot)?;
                    remove.push(snapshot.clone());
                }
            }
        }
        ensure!(
            remove.is_empty() || !survivors.is_empty(),
            "retention preview has no surviving snapshots"
        );
        let token = id();
        let keep_count = survivors.len();
        self.state.lock().unwrap().previews.insert(
            token.clone(),
            Preview {
                expires: now() + 300,
                revision: c.revision,
                repository: repo,
                profile_id: profile.id,
                repository_identity: identity,
                remove: remove.clone(),
                survivors,
            },
        );
        Ok(json!({"token":token,"remove":remove,"keep_count":keep_count}))
    }
    fn apply_retention(&self, c: &Config, token: &str, confirmation: &str) -> Result<Value> {
        ensure!(
            confirmation == "DELETE",
            "type DELETE to confirm the previewed snapshot deletion"
        );
        let preview = self
            .state
            .lock()
            .unwrap()
            .previews
            .remove(token)
            .context("preview token missing or already used")?;
        ensure!(
            preview.expires > now() && preview.revision == c.revision,
            "preview expired or configuration changed"
        );
        let repo = Self::repository(c, &preview.repository.id)?;
        ensure!(
            repo.location == preview.repository.location,
            "repository location changed"
        );
        ensure!(
            self.repository_identity(c, &repo)? == preview.repository_identity,
            "repository identity changed"
        );
        let snapshots = self.query(c, &repo, &strings(&["snapshots"]))?;
        let snapshots = snapshots
            .as_array()
            .context("unexpected snapshots response")?;
        let by_id: HashMap<_, _> = snapshots
            .iter()
            .map(|s| Ok((full_snapshot_id(s)?, s)))
            .collect::<Result<_>>()?;
        ensure!(
            preview.survivors.iter().all(|id| by_id.contains_key(id)),
            "preview survivors changed; create a new preview"
        );
        let tag = format!("ysynk-profile:{}", preview.profile_id);
        let mut ids = vec![];
        for candidate in &preview.remove {
            let id = full_snapshot_id(candidate)?;
            let current = by_id
                .get(&id)
                .context("preview candidate changed; create a new preview")?;
            ensure!(
                current["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some(&tag))),
                "preview candidate no longer carries the expected profile tag"
            );
            ensure!(
                !preview.survivors.contains(&id),
                "candidate overlaps survivor set"
            );
            ids.push(id);
        }
        if !ids.is_empty() {
            let mut args = strings(&["forget", "--"]);
            args.extend(ids.clone());
            let output = self.run(c, &repo, &args)?;
            restic::successful(&output)?;
        }
        Ok(
            json!({"removed_ids":ids,"pruned":false,"external_race_warning":"External restic changes between validation and deletion are not transactional"}),
        )
    }
    fn restore(
        &self,
        c: &Config,
        repo: &Repository,
        snapshot: &str,
        paths: &[String],
        target: &str,
    ) -> Result<Value> {
        config::snapshot(snapshot)?;
        config::absolute(target)?;
        let target_path = config::resolved(Path::new(target))?;
        ensure!(
            !config::overlaps(&target_path, &config::resolved(&self.directory)?),
            "restore target must not overlap app data"
        );
        for local in c
            .repositories
            .iter()
            .filter_map(|r| config::local_repository(&r.location))
        {
            ensure!(
                !config::overlaps(&target_path, &config::resolved(local)?),
                "restore target must not overlap a local repository"
            );
        }
        ensure!(paths.len() <= 1024, "too many restore paths");
        let mut args = strings(&["restore", snapshot, "--overwrite", "never", "--verify"]);
        for path in paths {
            restic_path(path)?;
            args.push("--include".into());
            args.push(literal_pattern(path));
        }
        // Fetch before creating output so a missing/locked keychain leaves no
        // restore directory. run() fetches again for the actual invocation.
        drop(credentials::required(self.credentials.as_ref(), repo)?);
        let output_dir = private_restore_directory(Path::new(target))?;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            args.extend(strings(&[
                "--target",
                &format!(
                    "/proc/{}/fd/{}/{}",
                    std::process::id(),
                    output_dir.as_raw_fd(),
                    Path::new(target)
                        .file_name()
                        .context("restore target must have a directory name")?
                        .to_str()
                        .context("invalid restore target name")?
                ),
            ]));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = output_dir;
            bail!("safe descriptor-based restore currently requires Linux");
        }
        let output = self
            .run(c, repo, &args)
            .context("restore failed; incomplete output is preserved in the requested target")?;
        ensure!(
            output.code == 0,
            "{}; incomplete output is preserved in the requested target",
            restic::exit_message(output.code)
        );
        Ok(json!({"target":target,"verified":true}))
    }
}
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
fn password_export_target(c: &Config, data: &Path, target: &Path) -> Result<PathBuf> {
    fn validate(c: &Config, data: &Path, target: &Path) -> Result<PathBuf> {
        config::absolute(target.to_str().context("invalid path")?)?;
        let parent = fs::canonicalize(target.parent().context("missing parent")?)?;
        ensure!(parent.is_dir(), "parent is not a directory");
        let target = parent.join(target.file_name().context("missing file name")?);
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("target already exists or is inaccessible"),
        }
        let mut protected = vec![config::resolved(data)?];
        for repo in c
            .repositories
            .iter()
            .filter_map(|r| config::local_repository(&r.location))
        {
            protected.push(config::resolved(repo)?);
        }
        for source in c.profiles.iter().flat_map(|p| &p.sources) {
            protected.push(config::resolved(Path::new(source))?);
        }
        ensure!(
            !protected.iter().any(|path| config::overlaps(path, &target)),
            "protected export location"
        );
        Ok(target)
    }
    validate(c, data, target).map_err(|_| anyhow::anyhow!("Choose a new password file in an existing private folder outside app data, repositories, and backup sources; existing files and symlinks are never overwritten"))
}
fn repository_config_id(value: &Value) -> Result<String> {
    let id = value["id"]
        .as_str()
        .context("repository identity missing")?;
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid repository identity"
    );
    Ok(id.to_owned())
}
fn full_snapshot_id(snapshot: &Value) -> Result<String> {
    let id = snapshot["id"].as_str().context("snapshot ID missing")?;
    config::snapshot(id)?;
    ensure!(id.len() == 64, "full snapshot ID required");
    Ok(id.into())
}
fn restic_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && config::text(path)
            && !path.split('/').any(|p| p == "." || p == ".."),
        "invalid absolute snapshot path"
    );
    Ok(())
}
fn literal_pattern(path: &str) -> String {
    path.chars()
        .flat_map(|c| {
            if "*?[]\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
fn request_scope(request: &Request) -> (&'static str, Option<String>, Option<String>) {
    match request {
        Request::Initialize { repository_id } => ("initialize", None, Some(repository_id.clone())),
        Request::Backup { profile_id } => ("backup", Some(profile_id.clone()), None),
        Request::Snapshots { repository_id } => ("snapshots", None, Some(repository_id.clone())),
        Request::ListFiles { repository_id, .. } => {
            ("list_files", None, Some(repository_id.clone()))
        }
        Request::Restore { repository_id, .. } => ("restore", None, Some(repository_id.clone())),
        Request::Stats { repository_id } => ("stats", None, Some(repository_id.clone())),
        Request::Check { repository_id } => ("check", None, Some(repository_id.clone())),
        Request::RetentionPreview {
            repository_id,
            profile_id,
        } => (
            "retention_preview",
            Some(profile_id.clone()),
            Some(repository_id.clone()),
        ),
        Request::ApplyRetention { .. } => ("apply_retention", None, None),
        Request::Prune { repository_id, .. } => ("prune", None, Some(repository_id.clone())),
        _ => ("invalid", None, None),
    }
}
#[cfg(target_os = "linux")]
fn private_restore_directory(target: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let mut parent = File::open("/")?;
    let components: Vec<_> = target.components().collect();
    ensure!(
        components.len() >= 2,
        "restore target must be a new directory"
    );
    for component in &components[1..components.len() - 1] {
        let name = CString::new(component.as_os_str().as_bytes())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "restore parent must exist and contain no symlinks");
        parent = unsafe { File::from_raw_fd(fd) };
    }
    // The descriptor pins the parent even if an ancestor is renamed. Its
    // entries must not be replaceable by another user while restic writes.
    // A trusted sticky directory (such as /tmp) protects our new owned entry.
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = parent.metadata()?;
        let trusted_owner = metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() };
        ensure!(
            trusted_owner && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
            "restore parent must be owned by this user or root and not writable by others unless sticky"
        );
    }
    let name = CString::new(components.last().unwrap().as_os_str().as_bytes())?;
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    ensure!(
        created == 0,
        "restore output must not already exist, and parent must be writable"
    );
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(
        fd >= 0,
        "could not open the newly created restore directory"
    );
    // Restic must see a real directory as the final component, not a /proc symlink.
    drop(unsafe { File::from_raw_fd(fd) });
    Ok(parent)
}
#[cfg(not(target_os = "linux"))]
fn private_restore_directory(_: &Path) -> Result<File> {
    bail!("safe descriptor-based restore currently requires Linux")
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
