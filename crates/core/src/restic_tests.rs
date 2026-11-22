use super::*;
use std::{fs, path::Path};

fn sink(password: &str) -> console::Sink {
    console::Sink {
        queue: Arc::new(std::sync::Mutex::new(console::Tail::default())),
        password: Arc::new(Zeroizing::new(password.into())),
    }
}

#[test]
fn version_probe_checks_the_program_name_and_minimum_version() {
    assert!(parse_version("restic 0.17.0").is_err());
    assert!(parse_version("other 1.0.0").is_err());
    assert!(parse_version("restic").is_err());
    assert_eq!(
        parse_version("restic 0.17.1 compiled with go on linux/amd64").unwrap(),
        "restic 0.17.1"
    );
}

#[test]
fn console_delivers_both_pipes_before_exit_and_drains_trailing_lines() {
    let dir = tempfile::tempdir().unwrap();
    let ack = dir.path().join("ack");
    let mut c = command("/usr/bin/python3");
    c.args([
        "-c",
        r#"import os,sys,time
print('stdout live', flush=True)
print('stderr live', file=sys.stderr, flush=True)
end=time.monotonic()+3
while not os.path.exists(sys.argv[1]):
    if time.monotonic()>end: sys.exit(9)
    time.sleep(.01)
sys.stdout.write('stdout tail')
sys.stderr.write('stderr tail')
"#,
    ])
    .arg(&ack);
    let mut lines = Vec::new();
    let output = run_live(
        c,
        &AtomicBool::new(false),
        None,
        Live {
            sink: sink("private-password"),
            stdout: true,
            callback: &mut |tail| {
                lines.extend(tail.lines);
                if lines.iter().any(|s| s == "stdout live")
                    && lines.iter().any(|s| s == "stderr live")
                {
                    fs::write(&ack, b"received before exit").unwrap();
                }
            },
        },
    )
    .unwrap();
    assert_eq!(output.code, 0);
    assert!(lines.iter().any(|s| s == "stdout tail"));
    assert!(lines.iter().any(|s| s == "stderr tail"));
}

#[test]
fn console_redacts_literal_credentials_before_control_sequences_can_consume_their_prefix() {
    let password = "a".repeat(64);
    let sink = sink(&password);
    let mut records = console::Records::new(sink.clone());
    // CSI parsing consumes its first alphabetic byte as a terminator. If it
    // runs first, only 63 password characters remain and full matching fails.
    records.feed(b"\x1b[");
    records.feed(password.as_bytes());
    records.feed(b"\n");
    let tail = sink.queue.lock().unwrap();
    assert!(!tail.lines.is_empty());
    assert!(tail.lines.iter().all(|line| !line.contains(&password[1..])));
}

#[test]
fn console_redacts_split_and_encoded_secrets_before_shortening_records() {
    let sink = sink("SECRET-boundary");
    let mut records = console::Records::new(sink.clone());
    records.feed(b"split SEC");
    records.feed(b"RET-boundary done\n");
    records.feed(format!("{}SECRET-boundary\n", "x".repeat(1018)).as_bytes());
    records
        .feed(br#"{"message_type":"verbose_status","action":"new","item":"\u0053ECRET-boundary"}"#);
    records.feed(b"\n\x1b[31mcolor\x1b[0m\x1b]0;hidden\x07\ntrailing");
    records.finish();
    let tail = sink.queue.lock().unwrap();
    assert!(tail.truncated);
    assert!(tail.lines.iter().all(|s| s.len() <= 1024
        && !s.contains("SECRET")
        && !s.contains("boundar")
        && !s.chars().any(char::is_control)));
    assert!(tail.lines.iter().any(|s| s == "new: [redacted]"));
    assert!(tail.lines.iter().any(|s| s == "color"));
    assert_eq!(tail.lines.last().unwrap(), "trailing");
}

#[test]
fn console_strips_control_sequences_before_final_redaction_and_truncates_at_utf8_boundaries() {
    let sink = sink("SECRET");
    let mut records = console::Records::new(sink.clone());
    records.feed(b"SEC\x1b[31mRET\x1b[0m\n");
    records.feed("\u{1f600}".repeat(300).as_bytes());
    records.finish();
    let tail = sink.queue.lock().unwrap();
    assert_eq!(tail.lines[0], "[redacted]");
    assert_eq!(tail.lines[1].len(), 1024);
    assert_eq!(tail.lines[1].chars().count(), 256);
    assert!(tail.truncated);
}

#[test]
fn console_redacts_ansi_wrapped_json_and_omits_malformed_unicode_escaped_records() {
    let sink = sink("SECRET");
    let escaped = r#"\u0053\u0045\u0043\u0052\u0045\u0054"#;
    let wrapped = format!(
        "\x1b[31m{{\"message_type\":\"verbose_status\",\"action\":\"new\",\"item\":\"{escaped}\"}}\x1b[0m\n"
    );
    let mut records = console::Records::new(sink.clone());
    for chunk in wrapped.as_bytes().chunks(3) {
        records.feed(chunk);
    }
    let malformed = format!("{{\"item\":\"{escaped}\"\n");
    for chunk in malformed.as_bytes().chunks(5) {
        records.feed(chunk);
    }
    records.feed(b"ordinary diagnostic\n");
    let tail = sink.queue.lock().unwrap();
    assert!(tail.truncated);
    assert_eq!(
        tail.lines,
        [
            "new: [redacted]",
            "Diagnostic record omitted: invalid escaped data",
            "ordinary diagnostic"
        ]
    );
    assert!(
        tail.lines
            .iter()
            .all(|line| !line.contains("SECRET") && !line.contains("\\u"))
    );
}

#[test]
fn console_discards_oversized_records_and_keeps_a_bounded_recent_tail() {
    let sink = sink("SECRET");
    let mut records = console::Records::new(sink.clone());
    records.feed(&vec![b'x'; console::RECORD_LIMIT]);
    records.feed(b"SECRET\n");
    for index in 0..10000 {
        records.feed(format!("{index}:{}\n", "y".repeat(1000)).as_bytes());
    }
    records.finish();
    let tail = sink.queue.lock().unwrap();
    assert!(tail.truncated);
    assert!(tail.lines.len() <= 200);
    assert!(tail.lines.iter().map(String::len).sum::<usize>() <= 65536);
    assert!(tail.lines.last().unwrap().starts_with("9999:"));
    assert!(
        !tail
            .lines
            .iter()
            .any(|s| s.contains('x') || s.contains("SECRET"))
    );
}

#[test]
fn query_stdout_is_not_forwarded_but_stderr_is_retained_on_failure() {
    let mut c = command("/usr/bin/python3");
    c.args(["-c", "import sys; print('[{\"path\":\"private-file\"}]'); print('diagnostic',file=sys.stderr); sys.exit(2)"]);
    let mut lines = Vec::new();
    let output = run_live(
        c,
        &AtomicBool::new(false),
        None,
        Live {
            sink: sink("SECRET"),
            stdout: false,
            callback: &mut |tail| lines.extend(tail.lines),
        },
    )
    .unwrap();
    assert_eq!(output.code, 2);
    assert_eq!(lines, ["diagnostic"]);
    assert!(
        std::str::from_utf8(&output.bytes)
            .unwrap()
            .contains("private-file")
    );
}

#[test]
fn cancellation_preserves_already_delivered_diagnostics() {
    let cancel = AtomicBool::new(false);
    let mut c = command("/usr/bin/python3");
    c.args([
        "-c",
        "import sys,time; print('before cancel',file=sys.stderr,flush=True); time.sleep(5)",
    ]);
    let mut lines = Vec::new();
    let started = Instant::now();
    let result = run_live(
        c,
        &cancel,
        None,
        Live {
            sink: sink("SECRET"),
            stdout: true,
            callback: &mut |tail| {
                lines.extend(tail.lines);
                cancel.store(true, Ordering::Release);
            },
        },
    );
    assert!(result.err().unwrap().to_string().contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(lines.iter().any(|line| line == "before cancel"));
}

fn diagnostic_flood() -> Command {
    let mut c = command("/usr/bin/python3");
    c.args([
        "-c",
        r#"import sys
for i in range(10000):
    print(str(i)+':'+('x'*1000))
print('recent final diagnostic',flush=True)
"#,
    ]);
    c
}

#[test]
fn streaming_stdout_can_exceed_query_limit_without_retaining_raw_bytes() {
    let mut recent = console::Tail::default();
    let output = run_live(
        diagnostic_flood(),
        &AtomicBool::new(false),
        None,
        Live {
            sink: sink("SECRET"),
            stdout: true,
            callback: &mut |tail| {
                recent.truncated |= tail.truncated;
                for line in tail.lines {
                    recent.push(line);
                }
            },
        },
    )
    .unwrap();
    assert_eq!(output.code, 0);
    assert!(output.bytes.is_empty());
    assert!(recent.truncated);
    assert!(recent.lines.len() <= 200);
    assert!(
        recent
            .lines
            .iter()
            .all(|line| line.len() <= 1024 && !line.contains("SECRET"))
    );
    assert!(recent.lines.iter().map(String::len).sum::<usize>() <= 65536);
    assert_eq!(recent.lines.last().unwrap(), "recent final diagnostic");
}

#[test]
fn query_stdout_still_fails_when_raw_result_exceeds_eight_mib() {
    let mut recent = console::Tail::default();
    let result = run_live(
        diagnostic_flood(),
        &AtomicBool::new(false),
        None,
        Live {
            sink: sink("SECRET"),
            stdout: false,
            callback: &mut |tail| {
                for line in tail.lines {
                    recent.push(line);
                }
            },
        },
    );
    assert!(result.err().unwrap().to_string().contains("size limit"));
    assert!(
        recent
            .lines
            .iter()
            .all(|line| !line.contains("recent final diagnostic") && !line.contains("SECRET"))
    );
}

#[test]
fn streaming_stdout_keeps_credential_guard_across_reads_and_oversized_records() {
    let mut c = command("/usr/bin/python3");
    c.args([
        "-c",
        r#"import sys,time
sys.stdout.write('x'*65536+'SEC')
sys.stdout.flush()
time.sleep(.08)
sys.stdout.write('RET\nafter discarded record\n')
sys.stdout.flush()
"#,
    ]);
    let mut recent = console::Tail::default();
    let result = run_live(
        c,
        &AtomicBool::new(false),
        None,
        Live {
            sink: sink("SECRET"),
            stdout: true,
            callback: &mut |tail| {
                recent.truncated |= tail.truncated;
                for line in tail.lines {
                    recent.push(line);
                }
            },
        },
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("response withheld")
    );
    assert!(recent.truncated);
    assert_eq!(recent.lines, ["after discarded record"]);
}

// The disposable helper exits normally, but its detached descendant retains
// pipes outside the process group. It has its own short lifetime as a fallback;
// the guard also kills it after both passing and failing assertions.
struct DetachedHelper {
    directory: tempfile::TempDir,
}
impl DetachedHelper {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn pid_file(&self) -> std::path::PathBuf {
        self.directory.path().join("pid")
    }

    fn command(&self, only_input: bool) -> Command {
        assert!(Path::new("/usr/bin/python3").is_file());
        let mut command = command("/usr/bin/python3");
        command.args([
            "-c",
            r#"import os, sys, time
pid = os.fork()
if pid == 0:
    os.setsid()
    if sys.argv[2] == 'input':
        os.close(1)
        os.close(2)
    with open(sys.argv[1], 'w') as output:
        output.write(str(os.getpid()))
    time.sleep(4)
    os._exit(0)
while not os.path.exists(sys.argv[1]):
    time.sleep(0.005)
print('restic 0.18.1', flush=True)
os._exit(0)
"#,
        ]);
        command.arg(self.pid_file());
        command.arg(if only_input { "input" } else { "output" });
        command
    }
}
impl Drop for DetachedHelper {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(self.pid_file())
            && let Ok(pid) = pid.parse::<i32>()
        {
            // Only the process created by this isolated fixture is signalled.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

#[test]
fn timeout_stops_detached_output_pipe_readers() {
    let helper = DetachedHelper::new();
    let started = Instant::now();
    let result = run(
        helper.command(false),
        None,
        &AtomicBool::new(false),
        Some(Duration::from_millis(200)),
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.err().unwrap().to_string().contains("timed out"));
}

#[test]
fn cancellation_stops_detached_output_pipe_readers() {
    let helper = DetachedHelper::new();
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(100));
            cancel.store(true, Ordering::Release);
        });
        run(helper.command(false), None, &cancel, None)
    });
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.err().unwrap().to_string().contains("cancelled"));
}

#[test]
fn timeout_stops_a_writer_when_a_detached_process_keeps_stdin_open() {
    let helper = DetachedHelper::new();
    let started = Instant::now();
    let result = run(
        helper.command(true),
        Some(vec![b'x'; 1024 * 1024]),
        &AtomicBool::new(false),
        Some(Duration::from_millis(200)),
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.err().unwrap().to_string().contains("timed out"));
}

#[test]
fn an_exited_process_cannot_leave_output_readers_open_indefinitely() {
    let helper = DetachedHelper::new();
    let started = Instant::now();
    let result = run(helper.command(false), None, &AtomicBool::new(false), None);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.err().unwrap().to_string().contains("pipes"));
}
