//! Boot, shutdown, and the attack tests, each in its own child process.
//!
//! Signals, the panic hook, the umask, and the single-instance lock are
//! process-wide, so every scenario runs in a fresh process: the test binary
//! runs itself again with `SYSTEM_SUITE_CHILD` naming the scenario, and the
//! parent checks the exit code, the output, and the files left behind.
//! Every scenario keeps its directories under its own temporary root
//! (`<NAME>_<KIND>_DIR`), so nothing touches the user's own directories.

#![cfg(all(
    feature = "kernel",
    feature = "sys",
    feature = "paths",
    feature = "env",
    feature = "log",
    feature = "signals"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    env, fs,
    io::{BufRead as _, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use system_suite::{ErrorKind, System, exit};

const CHILD: &str = "SYSTEM_SUITE_CHILD";
const ROOT: &str = "SYSTEM_SUITE_ROOT";
const WAIT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// The child side
// ---------------------------------------------------------------------------

/// The entry point of every child process. In the parent run it does
/// nothing.
#[test]
fn child() {
    let Ok(scenario) = env::var(CHILD) else {
        return;
    };
    let root = PathBuf::from(env::var(ROOT).expect("the parent sets the root"));
    let code = match scenario.as_str() {
        "serve" => serve(&root),
        "boot" => boot_only(&root),
        "env-file-missing" => env_file_missing(&root),
        "panic" => panic_after_boot(&root),
        other => panic!("unknown scenario {other}"),
    };
    std::process::exit(i32::from(code));
}

fn name() -> String {
    env::var("SYSTEM_SUITE_NAME").expect("the parent sets the name")
}

fn report(error: &system_suite::Error) -> u8 {
    let _code = error.report();
    exit_value(error.kind())
}

/// Boots, says READY, and runs until a stop.
fn serve(_root: &Path) -> u8 {
    let system = match System::builder(&name()).start() {
        Ok(system) => system,
        Err(error) => return report(&error),
    };
    println!("READY");
    let code = system.run();
    if code == std::process::ExitCode::SUCCESS {
        0
    } else {
        1
    }
}

/// Boots and shuts down at once; a boot error is reported with its code.
fn boot_only(_root: &Path) -> u8 {
    match System::builder(&name()).start() {
        Ok(system) => {
            println!("READY");
            let _code = system.shutdown();
            0
        }
        Err(error) => report(&error),
    }
}

fn env_file_missing(root: &Path) -> u8 {
    match System::builder(&name())
        .env_file(root.join("missing.env"))
        .start()
    {
        Ok(_system) => 0,
        Err(error) => report(&error),
    }
}

/// Boots, logs a field the logger must redact, and panics with a secret in
/// scope that must reach neither the log nor the crash report.
fn panic_after_boot(_root: &Path) -> u8 {
    let system = match System::builder(&name()).start() {
        Ok(system) => system,
        Err(error) => return report(&error),
    };
    let secret = String::from("hunter2-in-scope");
    system_suite::log::info!(system.log(), "connecting", password = "hunter2-field");
    std::hint::black_box(&secret);
    panic!("boom: the worker lost its database");
}

fn exit_value(kind: ErrorKind) -> u8 {
    match kind {
        ErrorKind::Setup => exit::USAGE,
        ErrorKind::Paths | ErrorKind::Logging => exit::CANTCREAT,
        ErrorKind::Environment => exit::CONFIG,
        ErrorKind::AlreadyRunning => exit::TEMPFAIL,
        ErrorKind::LockDirInsecure => exit::NOPERM,
        ErrorKind::Limits => exit::UNAVAILABLE,
        ErrorKind::Interrupted => exit::OK,
        ErrorKind::Io => exit::IOERR,
        _ => exit::SOFTWARE,
    }
}

// ---------------------------------------------------------------------------
// The parent side
// ---------------------------------------------------------------------------

/// A scenario's private directory tree and the variables that point the
/// application's directories into it.
struct Scenario {
    name: String,
    root: PathBuf,
    vars: Vec<(String, String)>,
}

impl Scenario {
    fn new(name: &str) -> Scenario {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = env::temp_dir().join(format!(
            "system-suite-it-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let prefix: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        let mut vars = Vec::new();
        for kind in [
            "DATA",
            "CONFIG",
            "CACHE",
            "LOGS",
            "STATE",
            "RUNTIME",
            "TEMP",
            "DOCUMENTS",
        ] {
            let dir = root.join(kind.to_ascii_lowercase());
            vars.push((format!("{prefix}_{kind}_DIR"), dir.display().to_string()));
        }
        Scenario {
            name: name.to_owned(),
            root,
            vars,
        }
    }

    fn prefix(&self) -> String {
        self.name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect()
    }

    fn set(&mut self, key: &str, value: &str) {
        let key = format!("{}_{key}", self.prefix());
        self.vars.retain(|(k, _)| *k != key);
        self.vars.push((key, value.to_owned()));
    }

    fn dir(&self, kind: &str) -> PathBuf {
        self.root.join(kind)
    }

    fn command(&self, scenario: &str) -> Command {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "child", "--nocapture", "--test-threads=1"])
            .env(CHILD, scenario)
            .env(ROOT, &self.root)
            .env("SYSTEM_SUITE_NAME", &self.name)
            .envs(self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        command
    }

    fn run(&self, scenario: &str) -> Output {
        let mut child = self
            .command(scenario)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if started.elapsed() > WAIT {
                drop(child.kill());
                drop(child.wait());
                panic!("scenario {scenario} did not finish in {WAIT:?}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Starts a long-running child and waits for its READY line. Returns
    /// the child and its log directory.
    fn spawn_ready(
        &self,
        scenario: &str,
        configure: impl FnOnce(&mut Command),
    ) -> (Child, PathBuf) {
        let mut command = self.command(scenario);
        command.stdout(Stdio::piped()).stderr(Stdio::inherit());
        configure(&mut command);
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        for line in BufReader::new(stdout).lines() {
            // libtest prints "test child ... " without a newline first.
            if line.unwrap().trim_end().ends_with("READY") {
                return (child, self.dir("logs"));
            }
        }
        drop(child.kill());
        drop(child.wait());
        panic!("scenario {scenario} ended before READY");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.root));
    }
}

fn wait(child: &mut Child) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if started.elapsed() > WAIT {
            drop(child.kill());
            panic!("child did not stop in {WAIT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn code(output: &Output) -> Option<i32> {
    output.status.code()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn log_text(logs: &Path, name: &str) -> String {
    fs::read_to_string(logs.join(format!("{name}.log"))).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Boot and shutdown
// ---------------------------------------------------------------------------

#[test]
fn test_boot_succeeds_and_shuts_down_with_zero() {
    let scenario = Scenario::new("it-boot");
    let output = scenario.run("boot");
    assert_eq!(code(&output), Some(0), "{}", text(&output.stderr));
    let log = log_text(&scenario.dir("logs"), "it-boot");
    for line in [
        "\"instance\"",
        "\"boot step\"",
        "\"ready\"",
        "\"stopping\"",
        "\"stopped\"",
    ] {
        assert!(log.contains(line), "missing {line} in the log:\n{log}");
    }
}

#[cfg(unix)]
#[test]
fn test_sigterm_stops_in_order_with_zero() {
    let scenario = Scenario::new("it-term");
    let (mut child, logs) = scenario.spawn_ready("serve", |_| {});
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let status = wait(&mut child);
    assert_eq!(status.code(), Some(0));
    let log = log_text(&logs, "it-term");
    assert!(log.contains("SIGTERM"), "{log}");
    let stopping = log.find("\"stopping\"").unwrap();
    let stopped = log.find("\"stopped\"").unwrap();
    assert!(
        stopping < stopped,
        "the shutdown order was not followed:\n{log}"
    );
}

#[cfg(unix)]
#[test]
fn test_sigint_stops_with_zero() {
    let scenario = Scenario::new("it-int");
    let (mut child, logs) = scenario.spawn_ready("serve", |_| {});
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(wait(&mut child).code(), Some(0));
    assert!(log_text(&logs, "it-int").contains("SIGINT"));
}

#[cfg(windows)]
#[test]
fn test_ctrl_break_stops_with_zero() {
    use std::os::windows::process::CommandExt as _;

    // CTRL_C cannot be sent to another process group; CTRL_BREAK can, and
    // reaches the kernel through the same console handler.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CTRL_BREAK_EVENT: u32 = 1;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
    }

    let scenario = Scenario::new("it-break");
    let (mut child, logs) = scenario.spawn_ready("serve", |command| {
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    });
    // SAFETY: GenerateConsoleCtrlEvent takes two integers and touches no
    // memory of ours; the group is the child's process id.
    let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) };
    if sent == 0 {
        // No console to send through (a detached CI runner): nothing to test.
        drop(child.kill());
        drop(child.wait());
        return;
    }
    assert_eq!(wait(&mut child).code(), Some(0));
    let log = log_text(&logs, "it-break");
    assert!(
        log.contains("CTRL_BREAK"),
        "logs={} log:
{log}",
        logs.display()
    );
}

// ---------------------------------------------------------------------------
// Attack tests
// ---------------------------------------------------------------------------

/// T1: a second copy fails with AlreadyRunning before it opens anything.
#[test]
fn test_t1_second_copy_is_refused() {
    let scenario = Scenario::new("it-twice");
    let (mut first, _) = scenario.spawn_ready("serve", |_| {});
    let second = scenario.run("boot");
    assert_eq!(code(&second), Some(i32::from(exit::TEMPFAIL)));
    let stderr = text(&second.stderr);
    assert!(stderr.contains("step 6 (guard)"), "{stderr}");
    assert!(stderr.contains("do:"), "{stderr}");
    drop(first.kill());
    drop(first.wait());
}

/// T3: directories are owner-only and files unreadable by others.
#[cfg(unix)]
#[test]
fn test_t3_files_are_not_readable_by_others() {
    use std::os::unix::fs::PermissionsExt as _;

    let scenario = Scenario::new("it-modes");
    let (mut child, logs) = scenario.spawn_ready("serve", |_| {});
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&logs) & 0o077, 0, "log directory {:o}", mode(&logs));
    let log = logs.join("it-modes.log");
    assert_eq!(mode(&log) & 0o037, 0, "log file {:o}", mode(&log));
    let lock_dir = scenario.dir("runtime");
    assert_eq!(
        mode(&lock_dir) & 0o077,
        0,
        "lock directory {:o}",
        mode(&lock_dir)
    );
    drop(child.kill());
    drop(child.wait());
}

/// T3 on Windows: the log and lock directories carry a protected access
/// control list for the current user, SYSTEM, and Administrators only.
#[cfg(windows)]
#[test]
fn test_t3_directories_are_private_on_windows() {
    let scenario = Scenario::new("it-acl");
    let (mut child, logs) = scenario.spawn_ready("serve", |_| {});
    for dir in [logs, scenario.dir("runtime")] {
        let dacl = windows_acl::dacl_of(&dir);
        assert!(dacl.starts_with("D:P"), "{} is not protected: {dacl}", dir.display());
        assert_eq!(dacl.matches("(A;").count(), 3, "{}: {dacl}", dir.display());
        for other in [";;;WD)", ";;;BU)", ";;;AU)", ";;;IU)"] {
            assert!(!dacl.contains(other), "{other} in {dacl}");
        }
    }
    drop(child.kill());
    drop(child.wait());
}

/// Reads a directory's DACL back from the system, in SDDL form.
#[cfg(windows)]
mod windows_acl {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt as _, path::Path};

    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 4;
    const SDDL_REVISION_1: u32 = 1;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetNamedSecurityInfoW(
            name: *const u16,
            object: u32,
            info: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor: *mut c_void,
            revision: u32,
            info: u32,
            string: *mut *mut u16,
            length: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub(super) fn dacl_of(path: &Path) -> String {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let mut dacl: *mut c_void = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and every out-pointer is valid
        // or null where the API allows it.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut dacl,
                std::ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        assert_eq!(status, 0, "GetNamedSecurityInfoW failed for {}", path.display());
        let mut string: *mut u16 = std::ptr::null_mut();
        let mut length = 0u32;
        // SAFETY: `descriptor` came from the call above; the out-pointers
        // are valid.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut string,
                &raw mut length,
            )
        };
        assert_ne!(ok, 0);
        // SAFETY: `string` holds `length` units, terminator included.
        let units = unsafe { std::slice::from_raw_parts(string, length as usize) };
        let text = String::from_utf16_lossy(units);
        // SAFETY: both were allocated by the system with `LocalAlloc`.
        unsafe {
            LocalFree(string.cast::<c_void>());
            LocalFree(descriptor);
        }
        text.trim_end_matches(char::from(0)).to_owned()
    }
}

/// T5 and T9: a panic leaves the panic in the log and a crash report, and
/// neither contains the secret.
#[test]
fn test_t5_t9_panic_leaves_a_report_without_secrets() {
    let scenario = Scenario::new("it-crash");
    let output = scenario.run("panic");
    assert_ne!(code(&output), Some(0));
    let logs = scenario.dir("logs");
    let log = log_text(&logs, "it-crash");
    assert!(log.contains("boom: the worker lost its database"), "{log}");
    assert!(log.contains("[REDACTED]"), "{log}");
    assert!(!log.contains("hunter2"), "a secret reached the log:\n{log}");
    let reports: Vec<PathBuf> = fs::read_dir(&logs)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = fs::read_to_string(&reports[0]).unwrap();
    assert!(
        report.contains("panic: boom: the worker lost its database"),
        "{report}"
    );
    assert!(report.contains("instance: "), "{report}");
    assert!(
        !report.contains("hunter2"),
        "a secret reached the crash report:\n{report}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&reports[0]).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

// ---------------------------------------------------------------------------
// Failure injection at each boot step that can fail (threat T4)
// ---------------------------------------------------------------------------

fn assert_boot_fails(output: &Output, step: &str, exit_code: u8) {
    let stderr = text(&output.stderr);
    assert_eq!(code(output), Some(i32::from(exit_code)), "{stderr}");
    assert!(stderr.contains(step), "expected {step} in:\n{stderr}");
    assert!(stderr.contains("do:"), "{stderr}");
}

#[test]
fn test_t4_paths_step_fails_on_a_relative_override() {
    let mut scenario = Scenario::new("it-paths");
    scenario.set("LOGS_DIR", "relative/logs");
    assert_boot_fails(&scenario.run("boot"), "step 3 (paths)", exit::CANTCREAT);
    assert!(
        !scenario.dir("runtime").exists(),
        "the guard ran after a failed step"
    );
}

#[test]
fn test_t4_environment_step_fails_on_a_missing_explicit_file() {
    let scenario = Scenario::new("it-envfile");
    assert_boot_fails(
        &scenario.run("env-file-missing"),
        "step 4 (environment)",
        exit::CONFIG,
    );
}

#[test]
fn test_t4_environment_step_fails_on_a_malformed_file() {
    let scenario = Scenario::new("it-envbad");
    let config = scenario.dir("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join(".env"), "this line has no equals sign\n").unwrap();
    assert_boot_fails(&scenario.run("boot"), "step 4 (environment)", exit::CONFIG);
}

#[cfg(unix)]
#[test]
fn test_t4_guard_step_fails_on_a_world_writable_lock_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let scenario = Scenario::new("it-lockdir");
    let runtime = scenario.dir("runtime");
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o777)).unwrap();
    assert_boot_fails(&scenario.run("boot"), "step 6 (guard)", exit::NOPERM);
}

#[test]
fn test_t4_directories_step_fails_when_a_file_is_in_the_way() {
    let mut scenario = Scenario::new("it-dirs");
    let blocker = scenario.root.join("a-file");
    fs::write(&blocker, "not a directory").unwrap();
    scenario.set("LOGS_DIR", &blocker.display().to_string());
    assert_boot_fails(
        &scenario.run("boot"),
        "step 9 (directories)",
        exit::CANTCREAT,
    );
}

#[test]
fn test_t4_logging_step_fails_when_the_log_file_is_a_directory() {
    let scenario = Scenario::new("it-logfile");
    fs::create_dir_all(scenario.dir("logs").join("it-logfile.log")).unwrap();
    let output = scenario.run("boot");
    assert_boot_fails(&output, "step 10 (logging)", exit::CANTCREAT);
}
