use crate::{config::LIMIT, models::Progress};
use anyhow::{Result, ensure};
use std::{
    io::{BufRead, Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(crate) struct Output {
    pub code: i32,
    pub bytes: Zeroizing<Vec<u8>>,
}
pub(crate) fn command(executable: &str) -> Command {
    let mut command = Command::new(executable);
    command.env_clear();
    // Never inherit RESTIC_*, cloud credentials, command overrides, or loader hooks.
    for name in [
        "HOME",
        "USER",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SSH_AUTH_SOCK",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(unix)]
    {
        command.env("PATH", "/usr/local/bin:/usr/bin:/bin");
    }
    #[cfg(not(unix))]
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}
#[cfg(unix)]
fn signal(pid: u32, signal: i32) {
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}
fn reader(
    mut pipe: impl Read + Send + 'static,
    overflow: Arc<AtomicBool>,
    retain: bool,
) -> thread::JoinHandle<std::io::Result<Zeroizing<Vec<u8>>>> {
    thread::spawn(move || {
        let mut output = Zeroizing::new(vec![]);
        let mut buffer = Zeroizing::new([0u8; 8192]);
        loop {
            let n = pipe.read(buffer.as_mut())?;
            if n == 0 {
                break;
            }
            if retain {
                if output.len() + n > LIMIT {
                    overflow.store(true, Ordering::Release);
                } else {
                    output.extend_from_slice(&buffer[..n]);
                }
            }
        }
        Ok(output)
    })
}
fn progress_record(value: &serde_json::Value) -> Option<Progress> {
    match value["message_type"].as_str()? {
        "status" => Some(Progress {
            percent: value["percent_done"]
                .as_f64()
                .filter(|v| v.is_finite())
                .map(|v| (v * 100.0).clamp(0.0, 100.0)),
            bytes_done: value["bytes_done"].as_u64().unwrap_or(0),
            total_bytes: value["total_bytes"].as_u64().unwrap_or(0),
            files_done: value["files_done"].as_u64().unwrap_or(0),
            total_files: value["total_files"].as_u64().unwrap_or(0),
        }),
        "summary" => Some(Progress {
            percent: None,
            bytes_done: value["total_bytes_processed"].as_u64().unwrap_or(0),
            total_bytes: 0,
            files_done: value["total_files_processed"].as_u64().unwrap_or(0),
            total_files: 0,
        }),
        _ => None,
    }
}
fn backup_reader(
    pipe: impl Read + Send + 'static,
    overflow: Arc<AtomicBool>,
    progress: SyncSender<Progress>,
) -> thread::JoinHandle<std::io::Result<Zeroizing<Vec<u8>>>> {
    thread::spawn(move || {
        let mut pipe = std::io::BufReader::new(pipe);
        let mut summary = Zeroizing::new(vec![]);
        loop {
            let mut line = Zeroizing::new(vec![]);
            let count = (&mut pipe).take(65537).read_until(b'\n', &mut line)?;
            if count == 0 {
                break;
            }
            if count > 65536 {
                overflow.store(true, Ordering::Release);
                break;
            }
            let value: serde_json::Value = serde_json::from_slice(&line).map_err(|_| {
                overflow.store(true, Ordering::Release);
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid backup status record",
                )
            })?;
            if let Some(update) = progress_record(&value) {
                let _ = progress.try_send(update);
            }
            if value["message_type"] == "summary" {
                *summary = std::mem::take(&mut *line);
            }
        }
        Ok(summary)
    })
}
pub(crate) fn run(
    command: Command,
    input: Option<Vec<u8>>,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
) -> Result<Output> {
    run_inner(command, input, cancel, timeout, None)
}
pub(crate) fn run_backup(
    command: Command,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Output> {
    run_inner(command, None, cancel, None, Some(progress))
}
fn run_inner(
    mut command: Command,
    input: Option<Vec<u8>>,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    mut progress: Option<&mut dyn FnMut(Progress)>,
) -> Result<Output> {
    ensure!(!cancel.load(Ordering::Acquire), "operation cancelled");
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|_| {
        anyhow::anyhow!("could not start executable; check installed path and permissions")
    })?;
    let pid = child.id();
    let overflow = Arc::new(AtomicBool::new(false));
    let (sender, updates) = mpsc::sync_channel(8);
    let stdout = if progress.is_some() {
        backup_reader(child.stdout.take().unwrap(), overflow.clone(), sender)
    } else {
        reader(child.stdout.take().unwrap(), overflow.clone(), true)
    };
    let stderr = reader(child.stderr.take().unwrap(), overflow.clone(), false);
    // Write independently so cancellation remains responsive.
    let writer = input.map(|input| {
        let mut pipe = child.stdin.take().unwrap();
        thread::spawn(move || pipe.write_all(&input))
    });
    let start = Instant::now();
    let mut interrupted = None;
    let mut timed_out = false;
    let status = loop {
        if let Some(callback) = progress.as_mut() {
            for update in updates.try_iter().take(8) {
                callback(update);
            }
        }
        let stop = cancel.load(Ordering::Acquire)
            || overflow.load(Ordering::Acquire)
            || timeout.is_some_and(|t| start.elapsed() >= t);
        if stop && interrupted.is_none() {
            timed_out = timeout.is_some_and(|t| start.elapsed() >= t);
            interrupted = Some(Instant::now());
            #[cfg(unix)]
            signal(pid, libc::SIGINT);
            #[cfg(not(unix))]
            {
                let _ = child.kill();
            }
        }
        if interrupted.is_some_and(|t| t.elapsed() >= Duration::from_millis(500)) {
            #[cfg(unix)]
            signal(pid, libc::SIGKILL);
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(30)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    // Reap pipe-holding descendants too (for example transport subprocesses).
    #[cfg(unix)]
    signal(pid, libc::SIGKILL);
    let out = stdout.join();
    let _ = stderr.join();
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    if let Some(callback) = progress.as_mut() {
        for update in updates.try_iter().take(8) {
            callback(update);
        }
    }
    let out = out
        .map_err(|_| anyhow::anyhow!("output reader failed"))?
        .map_err(|_| anyhow::anyhow!("could not read bounded subprocess output"))?;
    ensure!(!cancel.load(Ordering::Acquire), "operation cancelled");
    ensure!(
        !overflow.load(Ordering::Acquire),
        "command output exceeds its size limit (queries: 8 MiB, backup records: 64 KiB); narrow the query"
    );
    ensure!(!timed_out, "executable version probe timed out");
    let status = status?;
    Ok(Output {
        code: status.code().unwrap_or(130),
        bytes: out,
    })
}
pub(crate) fn successful(output: &Output) -> Result<()> {
    ensure!(output.code == 0, "{}", exit_message(output.code));
    Ok(())
}
pub(crate) fn exit_message(code: i32) -> &'static str {
    match code {
        0 => "Completed",
        3 => "Backup incomplete: some source data could not be read",
        10 => "Repository does not exist",
        11 => "Repository is locked by another operation; no stale locks were removed",
        12 => "Repository rejected the password",
        130 => "Operation cancelled",
        _ => "Restic failed; check repository access, source permissions and backend configuration",
    }
}
pub(crate) fn version(executable: &str) -> Result<String> {
    let mut c = command(executable);
    c.arg("version");
    let output = run(
        c,
        None,
        &AtomicBool::new(false),
        Some(Duration::from_secs(3)),
    )?;
    successful(&output)?;
    let s = std::str::from_utf8(&output.bytes)?;
    let v = s
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("unrecognized restic version"))?;
    let parts = v
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        parts.len() == 3 && (parts[0], parts[1], parts[2]) >= (0, 17, 1),
        "restic 0.17.1 or later is required"
    );
    Ok(format!("restic {v}"))
}
