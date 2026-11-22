mod destination;

use crate::{
    config,
    credentials::{self, NativeStore, Store},
    models::*,
    restic,
};
use anyhow::{Context, Result, bail, ensure};
use destination::{
    password_export_target, pinned_export_target, private_restore_directory, sync_export_parent,
    trusted_parent,
};
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

#[derive(Serialize, Deserialize)]
struct Persisted<J = Job> {
    history: Vec<J>,
    attempts: BTreeMap<String, u64>,
    active: Option<J>,
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
    status: Status,
    attempts: BTreeMap<String, u64>,
    previews: HashMap<String, Preview>,
    persistence_failed: bool,
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
        let config: Config = if config_path.try_exists()? {
            serde_json::from_slice(&config::read(&config_path)?)?
        } else {
            Config::default()
        };
        config::validate(&config)?;
        config::validate_overlap(&config, &directory)?;
        let saved: Persisted = if directory.join("state.json").try_exists()? {
            serde_json::from_slice(&config::read(&directory.join("state.json"))?)?
        } else {
            Persisted {
                history: vec![],
                attempts: BTreeMap::new(),
                active: None,
            }
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
        for job in &mut history {
            job.output.clear();
            job.output_truncated = false;
        }
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
                status: Status {
                    config,
                    active: None,
                    history,
                    restic_version: version,
                    setup_error,
                },
                attempts: saved.attempts,
                previews: HashMap::new(),
                persistence_failed: false,
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
        let saved = Persisted {
            history: state.status.history.iter().map(journal_entry).collect(),
            attempts: state.attempts.clone(),
            active: state.status.active.as_ref().map(journal_entry),
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
    fn persistence_failed(state: &mut State) {
        state.persistence_failed = true;
        state.previews.clear();
        state.status.setup_error = Some(
            "Could not durably save application data; restart the app before continuing".into(),
        );
    }
    pub fn is_busy(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.status.active.is_some() || state.password_export
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
            !state.persistence_failed,
            "Could not durably save application data; restart the app before continuing"
        );
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "App is shutting down; password export was not completed"
        );
        ensure!(
            state.status.config.revision == revision,
            "Configuration changed; reload before exporting a repository password"
        );
        Self::repository(&state.status.config, repository_id).map_err(|_| {
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
                state.status.active.is_none() && !state.password_export,
                "Another operation is running; wait before exporting a repository password"
            );
            state.password_export = true;
            (repo, state.status.config.clone(), PasswordExport(self))
        };
        let validated_target = password_export_target(&configuration, &self.directory, &target)?;
        // Pin the trusted parent before reading any secret. All subsequent
        // staging, cleanup and publication use this descriptor, not its pathname.
        let parent_handle = trusted_parent(&target).map_err(|_| anyhow::anyhow!(
            "Choose an existing trusted password folder without symlinks; safe password export requires Linux"
        ))?;
        let target = pinned_export_target(&parent_handle, &target)?;
        ensure!(
            password_export_target(&configuration, &self.directory, &target)? == validated_target,
            "Password export folder changed; select the destination again"
        );
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
        // The pinned parent remains authoritative even if an ancestor moves.
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
        // Publication is irreversible here. Report uncertain durability without
        // a path-based rollback that could remove a replaced or valid file.
        sync_export_parent(&parent_handle).map_err(|_| anyhow::anyhow!(
            "Password file was written, but its folder could not be synced; durability is uncertain. Verify the existing file before relying on it"
        ))?;
        Ok(())
    }
    pub fn request(self: &Arc<Self>, request: Request) -> Result<Value> {
        if matches!(request, Request::Cancel) {
            let _state = self.state.lock().unwrap();
            self.cancel.store(true, Ordering::Release);
            return Ok(Value::Null);
        }
        let mut state = self.state.lock().unwrap();
        self.request_locked(&mut state, request)
    }
    fn request_locked(self: &Arc<Self>, state: &mut State, request: Request) -> Result<Value> {
        Self::expire(state);
        if matches!(request, Request::Status) {
            return Ok(serde_json::to_value(&state.status)?);
        }
        self.ensure_idle(state)?;
        match request {
            Request::SaveConfig { config } => {
                ensure!(
                    config.restic_path == state.status.config.restic_path,
                    "Choose a restic executable in the native file picker to change it"
                );
                self.save_config(state, config)
            }
            request => {
                ensure!(
                    state.status.setup_error.is_none(),
                    "{}",
                    state
                        .status
                        .setup_error
                        .as_deref()
                        .unwrap_or("restic unavailable")
                );
                let (kind, profile_id, repository_id) = request_scope(&request);
                if let Some(id) = &repository_id {
                    ensure!(
                        state.status.config.repositories.iter().any(|r| &r.id == id),
                        "unknown repository"
                    );
                }
                if let Some(id) = &profile_id {
                    ensure!(
                        state.status.config.profiles.iter().any(|p| &p.id == id),
                        "unknown profile"
                    );
                }
                let job_id = id();
                state.status.active = Some(Job {
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
                    output: vec![],
                    output_truncated: false,
                    destinations: vec![],
                    result: None,
                });
                let previous_attempts = state.attempts.clone();
                if kind == "backup"
                    && let Some(id) = profile_id
                {
                    state.attempts.insert(id, now());
                }
                if self.persist(state).is_err() {
                    state.status.active = None;
                    state.attempts = previous_attempts;
                    // A post-rename failure may leave a saved active job. On
                    // restart it is conservatively reported as interrupted.
                    Self::persistence_failed(state);
                    bail!(
                        "Could not durably save job admission; restart the app before continuing"
                    );
                }
                self.cancel.store(false, Ordering::Release);
                let config = state.status.config.clone();
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
    fn ensure_idle(&self, state: &State) -> Result<()> {
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "app is shutting down"
        );
        ensure!(
            !state.persistence_failed,
            "Could not durably save application data; restart the app before continuing"
        );
        ensure!(
            !state.password_export,
            "A repository password export is running; wait for it to finish"
        );
        ensure!(
            state.status.active.is_none(),
            "another job is running; cancel it and wait for completion"
        );
        Ok(())
    }
    /// Only the native file picker supplies this path. Renderer configuration
    /// requests may edit other fields but cannot authorize a different engine.
    pub fn set_restic_executable(&self, revision: u64, selected: &Path) -> Result<Value> {
        let path = selected
            .canonicalize()
            .context("Could not resolve the selected executable")?;
        let metadata = path.metadata()?;
        ensure!(metadata.is_file(), "Select a regular executable file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o111 != 0,
                "Select an executable file"
            );
        }
        let mut state = self.state.lock().unwrap();
        self.ensure_idle(&state)?;
        ensure!(
            revision == state.status.config.revision,
            "configuration changed; select the executable again"
        );
        let mut config = state.status.config.clone();
        config.restic_path = path
            .to_str()
            .context("Executable path must be valid UTF-8")?
            .into();
        self.save_config(&mut state, config)
    }
    fn save_config(&self, state: &mut State, mut config: Config) -> Result<Value> {
        ensure!(
            config.revision == state.status.config.revision,
            "configuration changed; reload before saving"
        );
        config::validate(&config)?;
        config::validate_overlap(&config, &self.directory)?;
        for old in &state.status.config.repositories {
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
        let changed_path = config.restic_path != state.status.config.restic_path;
        config.revision = config
            .revision
            .checked_add(1)
            .context("configuration revision overflow")?;
        if config::write(
            &self.directory.join("config.json"),
            &serde_json::to_vec(&config)?,
            true,
        )
        .is_err()
        {
            // Rename may already have committed. Keep memory unchanged,
            // but never run it again until startup reloads durable data.
            Self::persistence_failed(state);
            bail!("Could not durably save configuration; restart the app before continuing");
        }
        state.status.config = config;
        state.previews.clear();
        if changed_path {
            state.status.restic_version = None;
            state.status.setup_error =
                Some("Restic executable changed; restart the app to validate its version".into());
        }
        Ok(serde_json::to_value(&state.status)?)
    }
    fn complete(&self, outcome: Result<Value>) {
        let mut state = self.state.lock().unwrap();
        let mut job = state.status.active.take().unwrap();
        job.finished_at = Some(now());
        if self.cancel.load(Ordering::Acquire) {
            job.state = "cancelled".into();
            job.message = if job.kind == "restore" {
                "Restore cancelled. Any incomplete output is preserved."
            } else {
                "Operation cancelled."
            }
            .into();
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
        state.status.history.insert(0, job);
        state.status.history.truncate(100);
        let mut budget = 0;
        for job in &mut state.status.history {
            if let Some(result) = &job.result {
                budget += serde_json::to_vec(result).map_or(config::LIMIT + 1, |b| b.len());
                if budget > config::LIMIT {
                    job.result = None;
                }
            }
        }
        if self.persist(&state).is_err() {
            // The job already ran: retain its result and completion attempt in
            // memory, but do not admit more work after uncertain persistence.
            Self::persistence_failed(&mut state);
        }
    }
    pub fn tick(self: &Arc<Self>) {
        let mut state = self.state.lock().unwrap();
        Self::expire(&mut state);
        if state.status.active.is_some()
            || state.password_export
            || self.closing.load(Ordering::Acquire)
            || state.status.setup_error.is_some()
        {
            return;
        }
        if let Some(profile_id) = due_profile(&state, now()) {
            let _ = self.request_locked(&mut state, Request::Backup { profile_id });
        }
    }
    /// Advance a desktop notification cursor and return newly completed
    /// operations' states in completion order.
    pub fn notification_states(&self, cursor: &mut Option<String>) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let history = &state.status.history;
        let count = history
            .iter()
            .position(|job| Some(&job.id) == cursor.as_ref())
            .unwrap_or(history.len());
        if count == 0 {
            return Vec::new();
        }
        *cursor = Some(history[0].id.clone());
        history[..count]
            .iter()
            .rev()
            .filter(|job| {
                matches!(
                    job.kind.as_str(),
                    "backup" | "restore" | "check" | "apply_retention" | "prune"
                )
            })
            .map(|job| job.state.clone())
            .collect()
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
        self.run_with_credential(c, repo, args, record)
    }
    fn run_with_credential(
        &self,
        c: &Config,
        repo: &Repository,
        args: &[String],
        record: credentials::Record,
    ) -> Result<restic::Output> {
        let mut command = restic::command(&c.restic_path);
        command
            .env("RESTIC_REPOSITORY", &repo.location)
            .env("RESTIC_PASSWORD", &record.password)
            .env("RESTIC_PROGRESS_FPS", "1");
        command
            .arg("--json")
            .arg("--cache-dir")
            .arg(self.directory.join("cache"))
            .args(args);
        let operation = args.first().map(String::as_str).unwrap_or("command");
        if operation == "backup"
            && let Some(job) = self.state.lock().unwrap().status.active.as_mut()
        {
            job.progress = None;
        }
        // Never copy query JSON, snapshot arrays or file listings into logs.
        let stdout = matches!(operation, "backup" | "restore" | "check" | "prune" | "init");
        self.console_line(format!("Starting {operation}"));
        let sink = restic::console::Sink {
            queue: Arc::new(Mutex::new(restic::console::Tail::default())),
            password: Arc::new(zeroize::Zeroizing::new(record.password.to_string())),
        };
        let mut progress = |progress| {
            if let Some(job) = self.state.lock().unwrap().status.active.as_mut() {
                job.progress = Some(progress);
            }
        };
        let mut callback = |tail: restic::console::Tail| {
            let mut state = self.state.lock().unwrap();
            if let Some(job) = state.status.active.as_mut() {
                let mut output = restic::console::Tail {
                    lines: std::mem::take(&mut job.output),
                    truncated: job.output_truncated || tail.truncated,
                };
                for line in tail.lines {
                    output.push(line);
                }
                job.output = output.lines;
                job.output_truncated = output.truncated;
            }
        };
        let output = restic::run_live(
            command,
            &self.cancel,
            if operation == "backup" {
                Some(&mut progress)
            } else {
                None
            },
            restic::Live {
                sink,
                stdout,
                callback: &mut callback,
            },
        );
        self.console_line(match &output {
            Ok(output) => format!(
                "Finished {operation}: {}",
                restic::exit_message(output.code)
            ),
            Err(_) => format!("Stopped {operation}: operation failed or was cancelled"),
        });
        let output = output?;
        ensure!(
            !output
                .bytes
                .windows(record.password.len())
                .any(|part| part == record.password.as_bytes()),
            "Restic output contained a credential; response withheld"
        );
        Ok(output)
    }
    fn console_line(&self, line: String) {
        if let Some(job) = self.state.lock().unwrap().status.active.as_mut() {
            let mut tail = restic::console::Tail {
                lines: std::mem::take(&mut job.output),
                truncated: job.output_truncated,
            };
            tail.push(line);
            job.output = tail.lines;
            job.output_truncated = tail.truncated;
        }
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
        let mut args = strings(&[
            "backup",
            "--verbose=2",
            "--tag",
            &format!("ysynk-profile:{}", profile.id),
        ]);
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
                    let state = match output.code {
                        0 => {
                            success += 1;
                            "success"
                        }
                        3 => {
                            partial += 1;
                            "partial"
                        }
                        _ => "failed",
                    };
                    if let Ok(summary) = serde_json::from_slice::<Value>(&output.bytes) {
                        snapshot_id = summary["snapshot_id"].as_str().map(str::to_owned);
                        if output.code == 0 {
                            let bytes = summary["total_bytes_processed"].as_u64().unwrap_or(0);
                            let files = summary["total_files_processed"].as_u64().unwrap_or(0);
                            let mut state = self.state.lock().unwrap();
                            state.status.active.as_mut().unwrap().progress = Some(Progress {
                                percent: Some(100.0),
                                bytes_done: bytes,
                                total_bytes: bytes,
                                files_done: files,
                                total_files: files,
                            });
                        }
                    }
                    (state, restic::exit_message(output.code).to_owned())
                }
                Err(error) => ("failed", error.to_string()),
            };
            self.state
                .lock()
                .unwrap()
                .status
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
        let job = state.status.active.as_mut().unwrap();
        job.state = if success == profile.repository_ids.len() {
            "success"
        } else if success > 0 || partial > 0 {
            "partial"
        } else {
            "failed"
        }
        .into();
        let total = profile.repository_ids.len();
        let failed = total - success - partial;
        job.message = format!(
            "{success} of {total} destinations completed, {partial} incomplete, {failed} failed."
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
        let output = self.run(c, &repo, &args)?;
        restic::successful(&output)?;
        // Restic 0.18 emits no JSON for forget when no snapshot group matches.
        // Accept that successful no-op only here, not for other JSON queries.
        let result: Value = if output.bytes.is_empty() {
            json!([])
        } else {
            serde_json::from_slice(&output.bytes)
                .map_err(|_| anyhow::anyhow!("unexpected restic retention response"))?
        };
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
        let mut state = self.state.lock().unwrap();
        // One bounded result, not an unbounded number of large five-minute
        // previews. Only a successful replacement supersedes the previous one.
        state.previews.clear();
        state.previews.insert(
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
            .context("preview token missing, superseded or already used")?;
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
        // Obtain the bound credential before creating any output.
        let record = credentials::required(self.credentials.as_ref(), repo)?;
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
            .run_with_credential(c, repo, &args, record)
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
/// The journal has an explicit allowlist so console paths and query results
/// stay in memory. Serializing this view never clones those larger payloads.
fn journal_entry(job: &Job) -> Value {
    json!({
        "id": job.id,
        "kind": job.kind,
        "label": job.label,
        "state": job.state,
        "started_at": job.started_at,
        "finished_at": job.finished_at,
        "profile_id": job.profile_id,
        "repository_id": job.repository_id,
        "message": job.message,
        "progress": job.progress,
        "destinations": job.destinations,
        "result": null,
    })
}
fn due_profile(state: &State, now: u64) -> Option<String> {
    state
        .status
        .config
        .profiles
        .iter()
        .filter(|p| p.schedule_minutes > 0)
        .map(|p| {
            (
                p,
                state
                    .attempts
                    .get(&p.id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(p.schedule_minutes.saturating_mul(60)),
            )
        })
        .filter(|(_, deadline)| *deadline <= now)
        .min_by_key(|(_, deadline)| *deadline)
        .map(|(p, _)| p.id.clone())
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
    let mut pattern = String::with_capacity(path.len());
    for c in path.chars() {
        if "*?[]\\".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod output_tests;
