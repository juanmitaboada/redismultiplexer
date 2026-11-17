//! What the program writes (M3-j = ja, A-F5d, M1F-9, M3R-36): a line is written whole in
//! one call, without colour codes when the stream is not a terminal, a write that fails
//! is dropped (the program goes on delivering), and a warning that can come once per
//! package is printed once per 10 s per thread with the count of the others.

mod common;

use common::*;
use std::process::Command;

/// A healthy target and one that is down: the down one gives every thread error lines on
/// stderr, the banner and the statistics go to stdout. With one of the two streams a pipe
/// nobody reads (`closed`), every package is still delivered, the stop is clean (exit 0,
/// pid file removed) and the final status counts every package.
fn delivers_with_a_stream_closed(test: &str, cell: Cell, closed: Closed) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let down = closed_port();
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        format!(
            "  - name: 't1'\n    ssl: false\n    hostname: '127.0.0.1'\n    port: {}\n    password: ''\n    channel: 'rmtest:down'\n{}",
            down, SHORT_TIMEOUTS
        ),
    ];
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn_closed(&cfg, closed);
    let n = 2000;
    let input: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &input);
    ctx.wait_drained(&src, &[(&t0, n as usize)]);
    p.stop(&ctx.pid_file());
    let status: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ctx.dir.join("rm.status")).unwrap()).unwrap();
    let since = &status["since_start"];
    assert_eq!(since["in"].as_u64(), Some(n), "{}", status);
    assert_eq!(since["out"].as_u64(), Some(n), "{}", status);
    let open = match closed {
        Closed::Stdout => p.stderr(),
        Closed::Stderr => p.stdout(),
    };
    assert!(!open.contains("panicked"), "{}", open);
}

fn stdout_closed(test: &str, cell: Cell) {
    delivers_with_a_stream_closed(test, cell, Closed::Stdout)
}

fn stderr_closed(test: &str, cell: Cell) {
    delivers_with_a_stream_closed(test, cell, Closed::Stderr)
}

matrix!(closed_stdout_keeps_delivering, stdout_closed, 2);
matrix!(closed_stderr_keeps_delivering, stderr_closed, 2);

/// `--version`, `--help` and the usage error with stdout or stderr closed: their exit
/// codes (0, 0, 2), no panic (1.0.8 and stage 4: exit 101)
#[test]
fn version_and_help_with_a_stream_closed() {
    for closed in [Closed::Stdout, Closed::Stderr] {
        for (args, code) in [(&["--version"][..], 0), (&["--help"][..], 0), (&[][..], 2)] {
            let mut cmd = Command::new(bin());
            cmd.args(args);
            match closed {
                Closed::Stdout => cmd.stdout(closed_pipe()),
                Closed::Stderr => cmd.stderr(closed_pipe()),
            };
            let out = cmd.output().unwrap();
            assert_eq!(
                out.status.code(),
                Some(code),
                "{:?} {:?}: {}",
                closed,
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

/// Neither stream is a terminal here: no colour code in either (M3-j = ja), and every line
/// of the statistics is one whole line (the head once, at its start)
#[test]
fn log_has_no_colour_when_not_a_terminal() {
    let mut ctx = Ctx::new("nocolour");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let down = closed_port();
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        format!(
            "  - name: 't1'\n    ssl: false\n    hostname: '127.0.0.1'\n    port: {}\n    password: ''\n    channel: 'rmtest:down'\n{}",
            down, SHORT_TIMEOUTS
        ),
    ];
    let cfg = ctx.config(&src, 2, "replicant", "", &clients);
    let mut p = ctx.spawn(&cfg);
    let input: Vec<String> = (0..100).map(seq_pkg).collect();
    ctx.push(&src, &input);
    ctx.wait_drained(&src, &[(&t0, 100)]);
    p.stop(&ctx.pid_file());
    let (out, err) = (p.stdout(), p.stderr());
    assert!(!out.contains('\x1b'), "{}", out);
    assert!(!err.contains('\x1b'), "{}", err);
    assert!(err.contains("Error while connecting"), "{}", err);
    let stats: Vec<&str> = out.lines().filter(|l| l.contains("Incoming:")).collect();
    assert!(!stats.is_empty(), "{}", out);
    for line in stats {
        assert_eq!(line.matches("RedisMultiplexer: ").count(), 1, "{}", line);
        assert!(
            line.contains("Outgoing:") && line.contains("Dropped:"),
            "{}",
            line
        );
        assert!(
            line.as_bytes()[..4].iter().all(u8::is_ascii_digit),
            "{}",
            line
        );
    }
}

/// With ordering, packages without a `ts` are delivered at once and warned about once per
/// 10 s per thread: one line at the first, then the count of the others when the thread
/// ends; the counts add up to the packages (M3-j = ja, plan 2.8)
#[test]
fn ordering_without_ts_is_counted_in_the_log() {
    let mut ctx = Ctx::new("nots");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let n = 5000u64;
    let input: Vec<String> = (0..n).map(|i| format!("no-ts-{}", i)).collect();
    ctx.push(&src, &input);
    let ordering =
        "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 1\nordering_limit: 64\n";
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        ordering,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, n as usize)]);
    p.stop(&ctx.pid_file());
    let err = p.stderr();
    let lines: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("Found a package without ordering information"))
        .collect();
    assert!(lines.len() <= 3, "{} lines:\n{}", lines.len(), err);
    // A line stands for one package, or for N with "(N since the last line)"
    let counted: u64 = lines
        .iter()
        .map(|l| match l.split_once(" (") {
            Some((_, rest)) => rest
                .split(' ')
                .next()
                .and_then(|k| k.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("{}", l)),
            None => 1,
        })
        .sum();
    assert_eq!(counted, n, "{}", err);
}

/// A pseudo-terminal for one of the program's streams: the program writes to the slave
/// side, a thread reads the master side until the program and every copy of the slave
/// are gone (EIO), so the terminal never fills up
struct Pty {
    slave: Option<std::os::fd::OwnedFd>,
    reader: Option<std::thread::JoinHandle<Vec<u8>>>,
}

impl Pty {
    fn open() -> Pty {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: openpty writes the two descriptors it opens; no name, termios or size
        let r = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(r, 0, "openpty: {}", std::io::Error::last_os_error());
        // SAFETY: both descriptors were just opened by openpty and are owned here alone
        let (master, slave) = unsafe {
            (
                std::fs::File::from_raw_fd(master),
                std::os::fd::OwnedFd::from_raw_fd(slave),
            )
        };
        let reader = std::thread::spawn(move || {
            let mut master = master;
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            // Linux answers EIO on the master once no slave is open: the end of the output
            while let Ok(n) = master.read(&mut buf) {
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            got
        });
        Pty {
            slave: Some(slave),
            reader: Some(reader),
        }
    }

    /// The slave side for the child; the parent's copy is dropped right after the spawn
    fn stdio(&self) -> std::process::Stdio {
        std::process::Stdio::from(self.slave.as_ref().unwrap().try_clone().unwrap())
    }

    /// What the program wrote, once it ended (carriage returns of the terminal removed)
    fn output(mut self) -> String {
        drop(self.slave.take());
        let got = self.reader.take().unwrap().join().unwrap();
        String::from_utf8_lossy(&got).replace('\r', "")
    }
}

/// Both streams on a terminal: every line is coloured, its colour code first and the
/// closing code before the newline (M3-j = ja; M3R-50 = R50a); the other side is
/// `log_has_no_colour_when_not_a_terminal`
#[test]
fn log_has_colours_on_a_terminal() {
    let mut ctx = Ctx::new("colour");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let down = closed_port();
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        format!(
            "  - name: 't1'\n    ssl: false\n    hostname: '127.0.0.1'\n    port: {}\n    password: ''\n    channel: 'rmtest:down'\n{}",
            down, SHORT_TIMEOUTS
        ),
    ];
    let cfg = ctx.config(&src, 2, "replicant", "", &clients);
    let (out, err) = (Pty::open(), Pty::open());
    let mut child = Command::new(bin())
        .arg(&cfg)
        .stdin(std::process::Stdio::null())
        .stdout(out.stdio())
        .stderr(err.stdio())
        .spawn()
        .unwrap();
    let input: Vec<String> = (0..100).map(seq_pkg).collect();
    ctx.push(&src, &input);
    ctx.wait_drained(&src, &[(&t0, 100)]);
    // SAFETY: a signal to the child this test started and has not waited for yet
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let status = child.wait().unwrap();
    assert!(status.success(), "{:?}", status);
    assert!(!ctx.pid_file().exists());
    let (out, err) = (out.output(), err.output());
    let reset = "\x1b[1;0m";
    let finished = out
        .lines()
        .find(|l| l.contains("Program finished!"))
        .unwrap_or_else(|| panic!("{}", out));
    assert!(finished.starts_with("\x1b[1;32m"), "{:?}", finished);
    assert!(finished.ends_with(reset), "{:?}", finished);
    let error = err
        .lines()
        .find(|l| l.contains("Error while connecting"))
        .unwrap_or_else(|| panic!("{}", err));
    assert!(error.starts_with("\x1b["), "{:?}", error);
    assert!(error.ends_with(reset), "{:?}", error);
    for line in out.lines().chain(err.lines()).filter(|l| !l.is_empty()) {
        assert!(
            line.starts_with("\x1b["),
            "a line without its colour: {:?}",
            line
        );
    }
}
