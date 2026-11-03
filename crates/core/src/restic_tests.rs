use super::*;
use std::{fs, path::Path};

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
