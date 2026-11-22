use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub revision: u64,
    pub restic_path: String,
    pub repositories: Vec<Repository>,
    pub profiles: Vec<Profile>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            revision: 0,
            restic_path: "/usr/bin/restic".into(),
            repositories: vec![],
            profiles: vec![],
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub id: String,
    pub label: String,
    pub location: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub sources: Vec<String>,
    pub excludes: Vec<String>,
    pub repository_ids: Vec<String>,
    pub schedule_minutes: u64,
    pub retention: Retention,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub keep_last: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
}
#[derive(Clone, Serialize)]
pub struct Status {
    pub config: Config,
    pub active: Option<Job>,
    pub history: Vec<Job>,
    pub restic_version: Option<String>,
    pub setup_error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub state: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub profile_id: Option<String>,
    pub repository_id: Option<String>,
    pub message: String,
    pub progress: Option<Progress>,
    #[serde(default)]
    pub output: Vec<String>,
    #[serde(default)]
    pub output_truncated: bool,
    pub destinations: Vec<DestinationResult>,
    pub result: Option<Value>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Progress {
    pub percent: Option<f64>,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub files_done: u64,
    pub total_files: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct DestinationResult {
    pub repository_id: String,
    pub state: String,
    pub message: String,
    pub snapshot_id: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    SaveConfig {
        config: Config,
    },
    Initialize {
        repository_id: String,
    },
    Backup {
        profile_id: String,
    },
    Snapshots {
        repository_id: String,
    },
    ListFiles {
        repository_id: String,
        snapshot_id: String,
        #[serde(default = "root")]
        path: String,
    },
    Restore {
        repository_id: String,
        snapshot_id: String,
        paths: Vec<String>,
        target: String,
    },
    Stats {
        repository_id: String,
    },
    Check {
        repository_id: String,
    },
    RetentionPreview {
        repository_id: String,
        profile_id: String,
    },
    ApplyRetention {
        token: String,
        confirmation: String,
    },
    Prune {
        repository_id: String,
        confirmation: String,
    },
    /// Cancel the displayed job only while it still owns the active slot.
    CancelJob {
        job_id: String,
    },
    /// Legacy request targeting whichever job is active at receipt.
    Cancel,
}
fn root() -> String {
    "/".into()
}
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
