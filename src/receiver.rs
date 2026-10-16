//! Background croc receive loop.
//!
//! # How it works
//!
//! croc's receive side cannot sit and wait: each run either finds a sender and
//! receives, or gives up. So a background thread runs croc again and again:
//!
//! | Mode         | Poll interval | Wait per run | Rate-limit back-off |
//! |--------------|---------------|--------------|---------------------|
//! | Local (LAN)  |  3 s          | croc exits on its own (~1 s) | n/a  |
//!
//! Local mode finds the sender by multicast discovery. Where that fails (e.g. macOS
//! blocking multicast, or networks that filter it) an optional sender IP is passed as
//! `--ip host[:port]` — croc then connects straight to the sender's built-in relay
//! (port 9009 by default) and still exits within ~0.5 s if nobody is sending.
//! | Own relay    | 10 s          |  8 s         | 60 s                |
//! | Public relay | 30 s          | 25 s         | 60 s                |
//!
//! The interval is measured from the *start* of each run. Public relays allow about
//! 30 joins per minute, hence the 30 s interval and the 60 s back-off; `--local` is the
//! default because it needs no relay at all.
//!
//! # What croc prints (verified against croc 11.5.4)
//!
//! Everything goes to **stderr**, separated by `\r` (progress bar) as well as `\n`:
//!
//! ```text
//! connecting...  looking for sender... | waiting for sender...     ← no sender yet
//! authenticating code...  opening transfer channels...           ← sender found
//! Receiving 2 files (28.6 MB)  …progress…                        ← transferring
//! ```
//!
//! A text transfer (`croc send --text …`) writes no file: stderr says
//! `Receiving text message (N B)` and stdout carries the exact text.
//!
//! With `--local` and no sender croc exits with status 1 after ~1 s; with a relay it
//! waits in the room indefinitely, so the wait limit above is what ends the run. The
//! limit only applies *before* a sender is found — a transfer in progress is never
//! killed. File names in the progress bar are truncated, so received files are found by
//! diffing the output folder before and after the run (`--rename` guarantees new names).

use std::{
    collections::HashSet,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use gtk4::glib;

// ── Timing ─────────────────────────────────────────────────────────────────────

const INTERVAL_LOCAL: Duration = Duration::from_secs(3);
const INTERVAL_OWN_RELAY: Duration = Duration::from_secs(10);
const INTERVAL_PUBLIC_RELAY: Duration = Duration::from_secs(30);

/// Safety net only — with `--local` croc normally exits by itself in about a second.
const WAIT_LOCAL: Duration = Duration::from_secs(10);
const WAIT_OWN_RELAY: Duration = Duration::from_secs(8);
const WAIT_PUBLIC_RELAY: Duration = Duration::from_secs(25);

const BACKOFF_RATE_LIMITED: Duration = Duration::from_secs(60);

/// How often the loop checks whether it has been asked to stop.
const TICK: Duration = Duration::from_millis(200);

/// `KOPICO_DEBUG=1` logs every croc run (command, exit status, output, outcome).
fn debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("KOPICO_DEBUG").is_some_and(|v| v != "0"))
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Where croc looks for a sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayMode {
    /// LAN only (`croc --local`). Default: fast poll, no internet, no rate limit.
    /// `sender_ip` (`host` or `host:port`) skips multicast discovery via `--ip`.
    Local { sender_ip: Option<String> },
    /// A self-hosted relay (`croc relay`), e.g. `myhost:9009`.
    OwnRelay { address: String, password: String },
    /// croc's public relays. Rate limited to ~30 joins/min.
    PublicRelay,
}

impl RelayMode {
    fn interval(&self) -> Duration {
        match self {
            Self::Local { .. } => INTERVAL_LOCAL,
            Self::OwnRelay { .. } => INTERVAL_OWN_RELAY,
            Self::PublicRelay => INTERVAL_PUBLIC_RELAY,
        }
    }

    fn max_wait(&self) -> Duration {
        match self {
            Self::Local { .. } => WAIT_LOCAL,
            Self::OwnRelay { .. } => WAIT_OWN_RELAY,
            Self::PublicRelay => WAIT_PUBLIC_RELAY,
        }
    }
}

/// Sent from the receive loop to the GTK main thread.
#[derive(Debug)]
pub enum ReceiverEvent {
    /// A transfer finished. Top-level names (files or folders) inside the output folder.
    Received(Vec<String>),
    /// A text message (`croc send --text …`), exactly as sent.
    Text(String),
    /// The loop gave up and stopped (e.g. croc is not installed).
    Failed(String),
}

/// A running receive loop. [`ReceiverHandle::stop`] (or dropping it) shuts it down
/// and kills any croc process still running.
pub struct ReceiverHandle {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReceiverHandle {
    /// Spawn the receive loop. `on_event` runs on the **GTK main thread** and may
    /// capture GTK objects.
    pub fn start(
        mode: RelayMode,
        out_dir: PathBuf,
        code: String,
        on_event: impl Fn(ReceiverEvent) + 'static,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let (tx, rx) = async_channel::unbounded::<ReceiverEvent>();

        let thread = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("croc-receive".into())
                .spawn(move || receive_loop(&running, &mode, &out_dir, &code, &tx))
                .expect("failed to spawn croc-receive thread")
        };

        // Wakes only when an event arrives; ends when the loop drops its sender.
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                on_event(event);
            }
        });

        Self { running, thread: Some(thread) }
    }

    /// Stop the loop and wait for it. Returns within about [`TICK`]: the loop kills a
    /// running croc as soon as it notices.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ReceiverHandle {
    fn drop(&mut self) {
        // Signal only — the thread still kills croc on its own, without blocking GTK.
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Default output folder: `~/Downloads/Kopico`.
pub fn default_out_dir() -> PathBuf {
    dirs_next::download_dir()
        .or_else(dirs_next::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Kopico")
}

/// `path` with the home folder shown as `~`, for display in the UI.
pub fn display_path(path: &Path) -> String {
    match dirs_next::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

// ── Receive loop (background thread) ───────────────────────────────────────────

fn receive_loop(
    running: &AtomicBool,
    mode: &RelayMode,
    out_dir: &Path,
    code: &str,
    events: &async_channel::Sender<ReceiverEvent>,
) {
    let fail = |message: String| {
        eprintln!("[croc-receive] {message}");
        let _ = events.send_blocking(ReceiverEvent::Failed(message));
    };

    let Some(croc) = find_croc() else {
        return fail("croc is not installed or not on PATH".into());
    };
    if let Err(err) = std::fs::create_dir_all(out_dir) {
        return fail(format!("cannot create {}: {err}", out_dir.display()));
    }

    while running.load(Ordering::Relaxed) {
        let started = Instant::now();
        let before = list_dir(out_dir);

        // Interval counts from the start of the run; a back-off counts from now
        let next_run = match run_croc(&croc, running, mode, out_dir, code) {
            Ok(Outcome::Received) => {
                let after = list_dir(out_dir);
                let mut new: Vec<String> = after.difference(&before).cloned().collect();
                new.sort();
                println!("[croc-receive] received {new:?}");
                if events.send_blocking(ReceiverEvent::Received(new)).is_err() {
                    return; // UI side is gone
                }
                started + mode.interval()
            }
            Ok(Outcome::ReceivedText(text)) => {
                // The text itself is never logged
                println!("[croc-receive] received text ({} bytes)", text.len());
                if events.send_blocking(ReceiverEvent::Text(text)).is_err() {
                    return;
                }
                started + mode.interval()
            }
            Ok(Outcome::NoSender) => started + mode.interval(),
            Ok(Outcome::RateLimited) => {
                eprintln!(
                    "[croc-receive] relay is rate limiting — backing off {}s",
                    BACKOFF_RATE_LIMITED.as_secs()
                );
                Instant::now() + BACKOFF_RATE_LIMITED
            }
            Ok(Outcome::Stopped) => return,
            Err(err) => {
                eprintln!("[croc-receive] failed to run croc: {err}");
                started + mode.interval()
            }
        };
        sleep_until(running, next_run);
    }
}

#[derive(Debug)]
enum Outcome {
    Received,
    ReceivedText(String),
    NoSender,
    RateLimited,
    Stopped,
}

/// What the stderr reader thread reports while croc runs.
enum Signal {
    SenderFound,
    RateLimited,
}

/// One croc receive attempt.
fn run_croc(
    croc: &Path,
    running: &AtomicBool,
    mode: &RelayMode,
    out_dir: &Path,
    code: &str,
) -> std::io::Result<Outcome> {
    let mut cmd = Command::new(croc);
    cmd.args(["--yes", "--ignore-stdin", "--rename", "--out"])
        .arg(out_dir)
        // Since croc 10, a code on the command line is refused on UNIX — use the env var
        .env("CROC_SECRET", code)
        // Don't let the user's shell environment override the chosen mode
        .env_remove("CROC_RELAY")
        .env_remove("CROC_RELAY6")
        .env_remove("CROC_PASS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match mode {
        // With an IP, run exactly what works by hand (`croc --ip host:port`): croc dials
        // the sender's built-in relay directly and never falls back to a public relay.
        RelayMode::Local { sender_ip: Some(ip) } => {
            cmd.args(["--ip", ip]);
        }
        RelayMode::Local { sender_ip: None } => {
            cmd.arg("--local");
        }
        RelayMode::OwnRelay { address, password } => {
            cmd.args(["--relay", address]);
            if !password.is_empty() {
                cmd.env("CROC_PASS", password); // env, not argv: keeps it out of `ps`
            }
        }
        RelayMode::PublicRelay => {}
    }

    if debug_enabled() {
        // Program + args only: `{cmd:?}` would include the env, i.e. CROC_SECRET/CROC_PASS
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
        eprintln!(
            "[croc-receive] run: {} {} (code: {} chars)",
            cmd.get_program().to_string_lossy(),
            args.join(" "),
            code.chars().count()
        );
    }
    let mut child = cmd.spawn()?;

    // Drain both pipes on their own threads so croc can never block on a full pipe.
    let stdout = child.stdout.take().expect("piped stdout");
    let stdout_reader = std::thread::spawn(move || read_all(stdout));

    let (signals_tx, signals) = mpsc::channel();
    let stderr = child.stderr.take().expect("piped stderr");
    let stderr_reader = std::thread::spawn(move || watch_stderr(stderr, signals_tx));

    let deadline = Instant::now() + mode.max_wait();
    let mut sender_found = false;

    let outcome = loop {
        if !running.load(Ordering::Relaxed) {
            break Outcome::Stopped;
        }
        // (an error here just means "not exited yet"; croc is always killed below)
        if let Ok(Some(status)) = child.try_wait() {
            // Pick up anything said just before exit (e.g. "Receiving …")
            sender_found |= drain_signals(&signals) == Some(true);
            break finished(status, sender_found);
        }
        match signals.recv_timeout(TICK) {
            Ok(Signal::SenderFound) => sender_found = true,
            Ok(Signal::RateLimited) => break Outcome::RateLimited,
            Err(_) if !sender_found && Instant::now() >= deadline => break Outcome::NoSender,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // stderr closed but croc not reaped yet — wait instead of spinning
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
        }
    };

    // No-op if croc already exited; otherwise stops a waiting/rate-limited/stopped run
    let _ = child.kill();
    let status = child.wait();
    let stderr_lines = stderr_reader.join().unwrap_or_default();
    let stdout_text = stdout_reader.join().unwrap_or_default();

    // A text transfer's content is on stdout, byte for byte (no trimming)
    let outcome = match outcome {
        Outcome::Received if stderr_lines.iter().any(|l| is_text_message(l)) => {
            Outcome::ReceivedText(stdout_text.clone())
        }
        other => other,
    };

    if debug_enabled() {
        let stdout_shown = match &outcome {
            Outcome::ReceivedText(text) => format!("<{} bytes of text, not shown>", text.len()),
            _ => stdout_text.trim().to_owned(),
        };
        let outcome_shown = match &outcome {
            Outcome::ReceivedText(_) => "ReceivedText".to_owned(),
            other => format!("{other:?}"),
        };
        eprintln!(
            "[croc-receive]   → {outcome_shown} ({status:?})\n[croc-receive]     stderr: {}\n[croc-receive]     stdout: {stdout_shown}",
            stderr_lines.join(" / "),
        );
    }
    if matches!(outcome, Outcome::ReceivedText(_)) {
        return Ok(outcome);
    }

    // croc prints connection errors on stdout, e.g.
    //   "could not connect to …: relay admission rate limited"  → back off
    //   "could not connect to : found no relay addresses"        → nobody sending (--local)
    //   "could not connect to …: … connection refused"             → nobody sending (--ip)
    let stdout_text = stdout_text.trim();
    if matches!(outcome, Outcome::NoSender) && is_rate_limited(stdout_text) {
        return Ok(Outcome::RateLimited);
    }
    if !stdout_text.is_empty() && !is_no_sender_message(stdout_text) {
        eprintln!("[croc-receive] croc: {stdout_text}");
    }
    Ok(outcome)
}

fn finished(status: ExitStatus, sender_found: bool) -> Outcome {
    if status.success() && sender_found {
        Outcome::Received
    } else {
        Outcome::NoSender
    }
}

/// Returns `Some(true)` if a pending signal said a sender was found.
fn drain_signals(signals: &mpsc::Receiver<Signal>) -> Option<bool> {
    let mut found = None;
    while let Ok(signal) = signals.try_recv() {
        if matches!(signal, Signal::SenderFound) {
            found = Some(true);
        }
    }
    found
}

/// Reads croc's stderr, splitting on `\r` and `\n`, and reports state changes.
/// Returns the distinct status lines (progress bars excluded) for debug logging.
fn watch_stderr(mut stderr: impl Read, signals: mpsc::Sender<Signal>) -> Vec<String> {
    let mut pending = String::new();
    let mut buf = [0u8; 4096];
    let mut found_sent = false;
    let mut lines: Vec<String> = Vec::new();

    while let Ok(n) = stderr.read(&mut buf) {
        if n == 0 {
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(pos) = pending.find(['\r', '\n']) {
            let segment: String = pending.drain(..=pos).collect();
            let segment = segment.trim();
            if !segment.is_empty()
                && !segment.contains('%')
                && lines.len() < 20
                && !lines.iter().any(|l| l == segment)
            {
                lines.push(segment.to_owned());
            }
            if is_rate_limited(segment) {
                let _ = signals.send(Signal::RateLimited);
            } else if !found_sent && is_sender_found(segment) {
                found_sent = true;
                let _ = signals.send(Signal::SenderFound);
            }
        }
        // Progress bars can be long without a separator; don't grow forever
        if pending.len() > 8192 {
            pending.clear();
        }
    }
    lines
}

/// stderr line announcing a text transfer: "Receiving text message (17 B)".
fn is_text_message(segment: &str) -> bool {
    segment.starts_with("Receiving text message")
}

fn is_sender_found(segment: &str) -> bool {
    segment.starts_with("authenticating code")
        || segment.starts_with("opening transfer channels")
        || segment.starts_with("Receiving")
}

/// stderr: "The relay is rate limiting reconnects; retrying in …"
/// stdout: "could not connect to host:port: relay admission rate limited"
/// Deliberately narrow: a file name or size like "(429 MB)" must not match.
fn is_rate_limited(segment: &str) -> bool {
    segment.to_ascii_lowercase().contains("rate limit")
}

/// Expected when nobody is sending; not worth logging on every poll.
fn is_no_sender_message(text: &str) -> bool {
    text.contains("found no relay addresses") || text.contains("connection refused")
}

/// Accepts `host`, `host:port` or `[ipv6]:port` — croc validates the rest.
pub fn is_valid_sender_ip(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.chars().any(char::is_whitespace)
        && !value.contains('/')
}

fn read_all(mut stream: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn list_dir(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                // croc's temporary files while writing
                .filter(|name| !name.starts_with(".croc"))
                .collect()
        })
        .unwrap_or_default()
}

/// croc on `$PATH`, or in the usual install locations. Apps started from Finder or
/// a desktop launcher often get a minimal `$PATH` without `/usr/local/bin` etc.
fn find_croc() -> Option<PathBuf> {
    let home = dirs_next::home_dir().unwrap_or_default();
    let path = std::env::var_os("PATH").unwrap_or_else(OsString::new);
    std::env::split_paths(&path)
        .chain([
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/local/bin"),
            PathBuf::from("/usr/bin"),
            home.join(".local/bin"),
            home.join("go/bin"),
        ])
        .map(|dir| dir.join("croc"))
        .find(|candidate| candidate.is_file())
}

fn sleep_until(running: &AtomicBool, until: Instant) {
    while running.load(Ordering::Relaxed) {
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(TICK));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sender_phases() {
        assert!(is_sender_found("authenticating code..."));
        assert!(is_sender_found("Receiving 2 files (28.6 MB)"));
        assert!(!is_sender_found("waiting for sender..."));
        assert!(!is_sender_found("looking for sender..."));
    }

    #[test]
    fn detects_text_message() {
        assert!(is_text_message("Receiving text message (17 B)"));
        assert!(!is_text_message("Receiving 2 files (28.6 MB)"));
        assert!(!is_text_message("Receiving 'text message.txt' (1 B)"));
    }

    #[test]
    fn sender_ip_validation() {
        assert!(is_valid_sender_ip("192.168.1.20"));
        assert!(is_valid_sender_ip("192.168.1.20:9009"));
        assert!(is_valid_sender_ip("[fe80::1]:9009"));
        assert!(is_valid_sender_ip("macbook.local"));
        assert!(!is_valid_sender_ip(""));
        assert!(!is_valid_sender_ip("192.168.1.20 9009"));
        assert!(!is_valid_sender_ip("http://192.168.1.20"));
    }

    #[test]
    fn rate_limit_is_narrow() {
        assert!(is_rate_limited(
            "The relay is rate limiting reconnects; retrying in 10s (attempt 1)"
        ));
        assert!(is_rate_limited("rate limited"));
        assert!(is_rate_limited(
            "could not connect to 127.0.0.1:9440: relay admission rate limited"
        ));
        assert!(!is_rate_limited("Receiving 1 files (429 MB)"));
        assert!(!is_rate_limited("too many.bin 100% |████| (1/1 B)"));
    }

    // ── End-to-end tests against a real local `croc relay` ────────────────────
    // Need croc installed and free local ports, so they don't run by default:
    //     cargo test -- --ignored --test-threads=1

    /// Kills the wrapped process (relay / sender) when the test ends, pass or fail.
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn croc() -> PathBuf {
        find_croc().expect("croc must be installed for --ignored tests")
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kopico-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn start_relay(base_port: u16, extra: &[&str]) -> (KillOnDrop, RelayMode) {
        let ports: Vec<String> = (base_port..base_port + 5).map(|p| p.to_string()).collect();
        let relay = Command::new(croc())
            .args(["relay", "--host", "127.0.0.1", "--ports", &ports.join(",")])
            .args(extra)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let mode = RelayMode::OwnRelay {
            address: format!("127.0.0.1:{base_port}"),
            password: String::new(),
        };
        (KillOnDrop(relay), mode)
    }

    #[test]
    #[ignore]
    fn own_relay_receives_file() {
        let (_relay, mode) = start_relay(9410, &[]);
        let src = temp_dir("send");
        let out = temp_dir("recv");
        std::fs::write(src.join("hello kopico.txt"), "hi").unwrap();
        // A file that already exists in the output folder must not be overwritten
        std::fs::write(out.join("hello kopico.txt"), "keep me").unwrap();

        let code = format!("kopico-e2e-{}", std::process::id());
        let RelayMode::OwnRelay { address, .. } = &mode else { unreachable!() };
        let _sender = KillOnDrop(
            Command::new(croc())
                .args(["--yes", "--ignore-stdin", "--relay", address, "send"])
                .arg(src.join("hello kopico.txt"))
                .env("CROC_SECRET", &code)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );

        let running = Arc::new(AtomicBool::new(true));
        let (tx, rx) = async_channel::unbounded();
        let loop_thread = {
            let (running, out, code) = (running.clone(), out.clone(), code.clone());
            std::thread::spawn(move || receive_loop(&running, &mode, &out, &code, &tx))
        };
        // Watchdog: give up after 60 s instead of hanging the test
        {
            let running = running.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(60));
                running.store(false, Ordering::Relaxed);
            });
        }

        let event = rx.recv_blocking().expect("no event within 60 s");
        running.store(false, Ordering::Relaxed);
        loop_thread.join().unwrap();

        let ReceiverEvent::Received(names) = event else { panic!("unexpected {event:?}") };
        assert_eq!(names.len(), 1, "exactly the new file is reported: {names:?}");
        assert_ne!(names[0], "hello kopico.txt", "--rename keeps the existing file");
        assert_eq!(std::fs::read_to_string(out.join("hello kopico.txt")).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(out.join(&names[0])).unwrap(), "hi");
    }

    #[test]
    #[ignore]
    fn local_mode_with_sender_ip_receives_file() {
        let src = temp_dir("ip-send");
        let out = temp_dir("ip-recv");
        std::fs::write(src.join("via ip.txt"), "direct").unwrap();
        let code = format!("kopico-ip-e2e-{}", std::process::id());

        // A plain `croc send` runs its own relay on port 9009 — that's what --ip reaches
        let mut sender = Command::new(croc())
            .args(["--yes", "--ignore-stdin", "send"])
            .arg(src.join("via ip.txt"))
            .env("CROC_SECRET", &code)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Wait until the sender is ready ("On the other computer, run: …")
        let mut stderr = sender.stderr.take().unwrap();
        let _sender = KillOnDrop(sender);
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 { break; }
            seen.extend_from_slice(&buf[..n]);
            if String::from_utf8_lossy(&seen).contains("On the other computer") { break; }
        }
        std::thread::spawn(move || read_all(stderr)); // keep draining

        let mode = RelayMode::Local { sender_ip: Some("127.0.0.1".into()) };
        let before = list_dir(&out);
        let running = AtomicBool::new(true);
        let started = Instant::now();
        let outcome = run_croc(&croc(), &running, &mode, &out, &code).unwrap();
        assert!(matches!(outcome, Outcome::Received), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        let new: Vec<_> = list_dir(&out).difference(&before).cloned().collect();
        assert_eq!(new, vec!["via ip.txt".to_string()]);

        // Nobody at the address: croc gives up quickly (connection refused) → NoSender
        let mode = RelayMode::Local { sender_ip: Some("127.0.0.1:9399".into()) };
        let started = Instant::now();
        let outcome = run_croc(&croc(), &running, &mode, &out, &code).unwrap();
        assert!(matches!(outcome, Outcome::NoSender), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    }

    #[test]
    #[ignore]
    fn text_message_is_received_exactly() {
        let out = temp_dir("text");
        let text = "line one\n  indented \"quoted\" $HOME ü€\nlast line\n";
        let code = format!("kopico-text-e2e-{}", std::process::id());

        let mut sender = Command::new(croc())
            .args(["--yes", "--ignore-stdin", "send", "--text", text])
            .env("CROC_SECRET", &code)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = sender.stderr.take().unwrap();
        let _sender = KillOnDrop(sender);
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 { break; }
            seen.extend_from_slice(&buf[..n]);
            if String::from_utf8_lossy(&seen).contains("On the other computer") { break; }
        }
        std::thread::spawn(move || read_all(stderr));

        let before = list_dir(&out);
        let mode = RelayMode::Local { sender_ip: Some("127.0.0.1".into()) };
        let outcome = run_croc(&croc(), &AtomicBool::new(true), &mode, &out, &code).unwrap();
        let Outcome::ReceivedText(received) = outcome else { panic!("{outcome:?}") };
        assert_eq!(received, text, "text arrives byte for byte");
        assert_eq!(list_dir(&out), before, "no file is written for text");
    }

    #[test]
    #[ignore]
    fn waiting_run_is_cut_off_and_stop_is_fast() {
        let (_relay, mode) = start_relay(9420, &[]);
        let out = temp_dir("wait");
        let running = AtomicBool::new(true);

        // Nobody sending: croc waits in the relay room until our limit ends the run
        let started = Instant::now();
        let outcome = run_croc(&croc(), &running, &mode, &out, "kopico-nobody-sends").unwrap();
        let took = started.elapsed();
        assert!(matches!(outcome, Outcome::NoSender), "{outcome:?}");
        assert!(took >= WAIT_OWN_RELAY && took < WAIT_OWN_RELAY + Duration::from_secs(3), "{took:?}");

        // Stop request while croc is waiting → returns within a couple of ticks
        let running = Arc::new(AtomicBool::new(true));
        {
            let running = running.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(1));
                running.store(false, Ordering::Relaxed);
            });
        }
        // (fresh code: re-joining the room of the croc killed above makes the relay drop
        // the connection, and croc then spends ~1 s exiting on its own)
        let started = Instant::now();
        let outcome = run_croc(&croc(), &running, &mode, &out, "kopico-nobody-sends-2").unwrap();
        assert!(matches!(outcome, Outcome::Stopped), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    #[ignore]
    fn rate_limit_is_detected() {
        // A relay that allows a single join per minute from our IP
        let (_relay, mode) = start_relay(9430, &["--source-join-limit", "1"]);
        let out = temp_dir("ratelimit");
        let running = AtomicBool::new(true);

        let mut outcomes = Vec::new();
        for _ in 0..3 {
            let outcome = run_croc(&croc(), &running, &mode, &out, "kopico-rate-limit").unwrap();
            let limited = matches!(outcome, Outcome::RateLimited);
            outcomes.push(outcome);
            if limited {
                break;
            }
        }
        assert!(
            outcomes.iter().any(|o| matches!(o, Outcome::RateLimited)),
            "no rate limit detected: {outcomes:?}"
        );
    }

    #[test]
    fn stderr_split_on_carriage_returns() {
        let (tx, rx) = mpsc::channel();
        let output = "connecting...\rwaiting for sender...\rauthenticating code...\r\
                      Receiving 1 files (13 B)\r x 100% |█| (13/13 B)\n";
        watch_stderr(output.as_bytes(), tx);
        let signals: Vec<_> = rx.try_iter().collect();
        assert_eq!(signals.len(), 1, "SenderFound is reported once");
        assert!(matches!(signals[0], Signal::SenderFound));
    }
}
