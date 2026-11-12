//! Shared harness for the integration tests.
//!
//! Every test drives the real binary against the Redis server given by
//! RM_TEST_REDIS_URL (default redis://127.0.0.1:6379), using keys under
//! `rmtest:<test>:<unique suffix>:` that are deleted when the test ends.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use redis::Commands;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const ENV_URL: &str = "RM_TEST_REDIS_URL";
pub const DEFAULT_URL: &str = "redis://127.0.0.1:6379";
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(15);
pub const EXIT_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_redismultiplexer")
}

/// Redis endpoint taken from RM_TEST_REDIS_URL, split into the fields the
/// program's YAML configuration expects.
pub struct Endpoint {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: u32,
    pub ssl: bool,
}

/// `%XX` sequences of a URL part decoded, as the crate does for the test's own connection
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).expect("URL part is not UTF-8 once decoded")
}

impl Endpoint {
    /// The `username` and `database` lines, only when the test URL sets them, so the
    /// files stay as 1.0.8 wrote them otherwise
    pub fn access(&self, indent: &str) -> String {
        let mut out = String::new();
        if !self.username.is_empty() {
            out += &format!("{}username: {}\n", indent, q(&self.username));
        }
        if self.database != 0 {
            out += &format!("{}database: {}\n", indent, self.database);
        }
        out
    }

    pub fn from_env() -> Endpoint {
        let url = std::env::var(ENV_URL).unwrap_or_else(|_| DEFAULT_URL.to_string());
        let (ssl, rest) = if let Some(r) = url.strip_prefix("rediss://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("redis://") {
            (false, r)
        } else {
            panic!(
                "{} must start with redis:// or rediss://, got '{}'",
                ENV_URL, url
            );
        };
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a, p),
            None => (rest, ""),
        };
        // The configuration selects the database and passes user and password as written
        // (M3-i): the test URL's percent-encoded parts are decoded for the YAML
        let database = match path {
            "" => 0,
            p => p
                .parse()
                .unwrap_or_else(|_| panic!("{}: bad database '/{}'", ENV_URL, p)),
        };
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (u, h),
            None => ("", authority),
        };
        let (username, password) = match userinfo.split_once(':') {
            Some((u, p)) => (percent_decoded(u), percent_decoded(p)),
            None => (percent_decoded(userinfo), String::new()),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>()
                    .unwrap_or_else(|_| panic!("{}: bad port in '{}'", ENV_URL, url)),
            ),
            None => (hostport.to_string(), 6379),
        };
        Endpoint {
            url,
            host,
            port,
            username,
            password,
            database,
            ssl,
        }
    }
}

/// Per-test context: Redis connection, unique key prefix, temp dir.
pub struct Ctx {
    pub ep: Endpoint,
    pub prefix: String,
    pub dir: PathBuf,
    con: redis::Connection,
    keys: Vec<String>,
}

impl Ctx {
    pub fn new(test: &str) -> Ctx {
        // A-C4: in debug builds an unlimited client panics on its second package
        // (src/main.rs, can_send: `client.packages -= 1`). Remove this guard in M3.
        if cfg!(debug_assertions) {
            panic!("integration tests need a release build until A-C4 is fixed (M3): run `cargo test --release`");
        }
        let ep = Endpoint::from_env();
        let client = redis::Client::open(ep.url.as_str())
            .unwrap_or_else(|e| panic!("{}='{}' is not a valid Redis URL: {}", ENV_URL, ep.url, e));
        let mut con = client
            .get_connection_with_timeout(Duration::from_secs(2))
            .unwrap_or_else(|e| {
                panic!(
                    "Redis not reachable at {} ({}); set {} to a reachable server",
                    ep.url, e, ENV_URL
                )
            });
        let pong: String = redis::cmd("PING")
            .query(&mut con)
            .unwrap_or_else(|e| panic!("PING failed at {} ({}); check {}", ep.url, e, ENV_URL));
        assert_eq!(pong, "PONG");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suffix = format!("{}-{}", std::process::id(), nanos);
        let prefix = format!("rmtest:{}:{}", test, suffix);
        let dir = std::env::temp_dir().join(format!("rmtest-{}-{}", test, suffix));
        fs::create_dir_all(&dir).unwrap();
        Ctx {
            ep,
            prefix,
            dir,
            con,
            keys: Vec::new(),
        }
    }

    /// Full key name for `name`, registered for cleanup.
    pub fn key(&mut self, name: &str) -> String {
        let k = format!("{}:{}", self.prefix, name);
        if !self.keys.contains(&k) {
            self.keys.push(k.clone());
        }
        k
    }

    pub fn push(&mut self, key: &str, items: &[String]) {
        for chunk in items.chunks(1000) {
            let _: i64 = self.con.rpush(key, chunk.to_vec()).unwrap();
        }
    }

    pub fn push_bytes(&mut self, key: &str, items: &[&[u8]]) {
        let mut cmd = redis::cmd("RPUSH");
        cmd.arg(key);
        for item in items {
            cmd.arg(*item);
        }
        let _: i64 = cmd.query(&mut self.con).unwrap();
    }

    pub fn llen(&mut self, key: &str) -> usize {
        let n: i64 = self.con.llen(key).unwrap();
        n as usize
    }

    pub fn list(&mut self, key: &str) -> Vec<String> {
        self.con.lrange(key, 0, -1).unwrap()
    }

    /// Any command on the test connection, e.g. `ctx.cmd(&["INFO", "stats"])`.
    #[allow(dead_code)]
    pub fn cmd<T: redis::FromRedisValue>(&mut self, args: &[&str]) -> T {
        let mut c = redis::cmd(args[0]);
        for a in &args[1..] {
            c.arg(*a);
        }
        c.query(&mut self.con)
            .unwrap_or_else(|e| panic!("{:?} failed: {}", args, e))
    }

    #[allow(dead_code)]
    pub fn list_bytes(&mut self, key: &str) -> Vec<Vec<u8>> {
        self.con.lrange(key, 0, -1).unwrap()
    }

    pub fn ltrim(&mut self, key: &str, start: isize, stop: isize) {
        let _: () = self.con.ltrim(key, start, stop).unwrap();
    }

    /// YAML for one client (target) entry; `extra` holds already indented
    /// option lines such as "    filter: 'x'\n".
    pub fn client_yaml(&self, name: &str, key: &str, extra: &str) -> String {
        format!(
            "  - name: {}\n    ssl: {}\n    hostname: {}\n    port: {}\n    password: {}\n{}    channel: {}\n{}",
            q(name),
            self.ep.ssl,
            q(&self.ep.host),
            self.ep.port,
            q(&self.ep.password),
            self.ep.access("    "),
            q(key),
            extra
        )
    }

    /// YAML for a client (target) entry on 127.0.0.1:`port`, without password or TLS
    /// (a SlowTarget).
    pub fn client_yaml_at(&self, name: &str, key: &str, port: u16) -> String {
        format!(
            "  - name: {}\n    ssl: false\n    hostname: '127.0.0.1'\n    port: {}\n    password: ''\n    channel: {}\n",
            q(name),
            port,
            q(key)
        )
    }

    /// Writes a full configuration and returns its path. `extra` holds root
    /// option lines; `clients` are entries built with `client_yaml`.
    pub fn config(
        &self,
        src: &str,
        children: u16,
        mode: &str,
        extra: &str,
        clients: &[String],
    ) -> PathBuf {
        let yaml = format!(
            "name: 'source'\nssl: {}\nhostname: {}\nport: {}\npassword: {}\n{}channel: {}\nchildren: {}\nmode: {}\npid: {}\nstatus: {}\n{}clients:\n{}",
            self.ep.ssl,
            q(&self.ep.host),
            self.ep.port,
            q(&self.ep.password),
            self.ep.access(""),
            q(src),
            children,
            q(mode),
            q(self.pid_file().to_str().unwrap()),
            q(self.dir.join("rm.status").to_str().unwrap()),
            extra,
            clients.concat()
        );
        let path = self.dir.join("rm.yaml");
        fs::write(&path, yaml).unwrap();
        path
    }

    pub fn pid_file(&self) -> PathBuf {
        self.dir.join("rm.pid")
    }

    pub fn spawn(&self, config: &Path) -> Proc {
        self.spawn_cmd(Command::new(bin()).arg(config))
    }

    /// As spawn, with one more environment variable (a CA for a TLS target)
    pub fn spawn_env(&self, config: &Path, key: &str, value: &str) -> Proc {
        self.spawn_cmd(Command::new(bin()).arg(config).env(key, value))
    }

    /// As spawn, but with SIGHUP ignored when the program starts, as nohup leaves it.
    pub fn spawn_hup_ignored(&self, config: &Path) -> Proc {
        // `trap '' HUP` sets the ignore disposition, which exec keeps
        let script = "trap '' HUP; exec \"$0\" \"$1\"";
        self.spawn_cmd(Command::new("sh").args(["-c", script, bin()]).arg(config))
    }

    fn spawn_cmd(&self, cmd: &mut Command) -> Proc {
        let out = self.dir.join("stdout.log");
        let err = self.dir.join("stderr.log");
        // The program keeps an inherited SIG_IGN for SIGHUP (M3F-14), so a suite started
        // under nohup would see every SIGHUP test fail (M3R-16): start it with the default
        // disposition. spawn_hup_ignored sets the ignore again in its shell.
        // SAFETY: signal() is async-signal-safe, the only call between fork and exec
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(cmd, || {
                libc::signal(libc::SIGHUP, libc::SIG_DFL);
                Ok(())
            });
        }
        let child = cmd
            .stdin(Stdio::null())
            .stdout(fs::File::create(&out).unwrap())
            .stderr(fs::File::create(&err).unwrap())
            .spawn()
            .expect("failed to start redismultiplexer");
        Proc {
            child,
            out,
            err,
            status: None,
        }
    }

    /// Polls until `cond` holds or DRAIN_TIMEOUT passes; panics with `what`.
    pub fn wait_for(&mut self, what: &str, mut cond: impl FnMut(&mut Ctx) -> bool) {
        let start = Instant::now();
        while !cond(self) {
            if start.elapsed() > DRAIN_TIMEOUT {
                panic!("timeout after {:?} waiting for: {}", DRAIN_TIMEOUT, what);
            }
            thread::sleep(POLL);
        }
    }

    /// Waits until `src` is empty and every (key, count) target holds `count`.
    pub fn wait_drained(&mut self, src: &str, targets: &[(&str, usize)]) {
        let desc = format!("source {} empty and targets {:?}", src, targets);
        self.wait_for(&desc, |c| {
            c.llen(src) == 0 && targets.iter().all(|(k, n)| c.llen(k) == *n)
        });
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        if !self.keys.is_empty() {
            let _: Result<i64, _> = self.con.del(self.keys.clone());
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A running redismultiplexer process.
pub struct Proc {
    child: Child,
    out: PathBuf,
    err: PathBuf,
    status: Option<ExitStatus>,
}

impl Proc {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn sigint(&self) {
        self.signal("INT");
    }

    /// Sends a signal by name (INT, TERM, HUP).
    pub fn signal(&self, sig: &str) {
        // Shell builtin kill: no extra crate and no dependency on procps.
        let st = Command::new("sh")
            .args(["-c", "kill -$0 \"$1\"", sig, &self.pid().to_string()])
            .status()
            .unwrap();
        assert!(st.success(), "kill -{} {} failed", sig, self.pid());
    }

    pub fn wait_exit(&mut self) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                self.status = Some(st);
                return st;
            }
            if start.elapsed() > EXIT_TIMEOUT {
                panic!(
                    "process {} did not exit within {:?}",
                    self.pid(),
                    EXIT_TIMEOUT
                );
            }
            thread::sleep(POLL);
        }
    }

    /// SIGINT, then asserts exit status 0 and pid file removal.
    pub fn stop(&mut self, pid_file: &Path) {
        self.stop_with(pid_file, "INT");
    }

    /// The signal `sig`, then asserts exit status 0 and pid file removal.
    pub fn stop_with(&mut self, pid_file: &Path, sig: &str) {
        self.signal(sig);
        let st = self.wait_exit();
        assert!(st.success(), "exit status after SIG{}: {:?}", sig, st);
        assert!(!pid_file.exists(), "pid file {:?} left behind", pid_file);
    }

    pub fn stdout(&self) -> String {
        fs::read_to_string(&self.out).unwrap_or_default()
    }

    pub fn stderr(&self) -> String {
        fs::read_to_string(&self.err).unwrap_or_default()
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if thread::panicking() {
            eprintln!("--- redismultiplexer stdout ---\n{}", self.stdout());
            eprintln!("--- redismultiplexer stderr ---\n{}", self.stderr());
        }
    }
}

/// YAML single-quoted scalar.
pub fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `n` distinct packages: "<tag>-00000" ...
pub fn packages(tag: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{}-{:05}", tag, i)).collect()
}

/// Client lines with short timeouts for a target that never answers (S4-6, M3R-22): the
/// failure tests need not wait out the 2 s and 5 s defaults
pub const SHORT_TIMEOUTS: &str = "    connect_timeout: 0.2\n    command_timeout: 0.5\n";

/// Ordering with a 1 s buffer on the `ts` field of `seq_pkg` packages.
pub const ORDERING_1S: &str =
    "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 1\nordering_limit: 200\n";

/// Package number `seq` of a stop-under-load run; the number is also its timestamp.
pub fn seq_pkg(seq: u64) -> String {
    format!("{{\"ts\": {}, \"v\": \"x\"}}", seq)
}

/// The `ts` of a package that holds `"ts": N` anywhere (seq_pkg, or one with a prefix)
pub fn ts_in(p: &[u8]) -> u64 {
    let at = p
        .windows(6)
        .position(|w| w == b"\"ts\": ")
        .expect("package without ts")
        + 6;
    let digits: Vec<u8> = p[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .copied()
        .collect();
    String::from_utf8(digits).unwrap().parse().unwrap()
}

/// Panics unless the packages of one target are in increasing `ts` order
pub fn assert_ts_increasing(what: &str, list: &[Vec<u8>]) {
    let ts: Vec<u64> = list.iter().map(|p| ts_in(p)).collect();
    assert!(
        ts.windows(2).all(|w| w[0] < w[1]),
        "{}: out of order: {:?}",
        what,
        ts
    );
}

fn seq_of(p: &str) -> u64 {
    p[7..p.find(',').unwrap()].parse().unwrap()
}

/// Result of `stop_under_load`: sequence numbers delivered (in target order) and left in
/// the source (in source order).
pub struct StopRun {
    pub delivered: Vec<u64>,
    pub left: Vec<u64>,
}

/// Stop under load (M2R-1, M2R-7, M3R-3). A producer thread keeps pushing numbered
/// packages into the source while the program runs, SIGINT comes `run_for` after the
/// first delivery with the producer still pushing, and once the program has exited cleanly every package pushed
/// must be either delivered or still in the source, exactly once: the source keeps the
/// newest ones, in order, and the target holds all the older ones. The producer keeps the
/// source busy at SIGINT however fast the build is, so the test does not depend on a
/// package count.
pub fn stop_under_load(test: &str, children: u16, ordering: bool, run_for: Duration) -> StopRun {
    stop_under_load_with(test, children, ordering, run_for, "INT")
}

/// `stop_under_load` with the stop signal `sig` (INT, TERM, HUP).
pub fn stop_under_load_with(
    test: &str,
    children: u16,
    ordering: bool,
    run_for: Duration,
    sig: &str,
) -> StopRun {
    let cell = Cell {
        ordering,
        spreader: false,
        children,
    };
    stop_under_load_cell(test, cell, false, run_for, sig)
}

/// Stop under load for a matrix cell. Replicant delivers to one target, spreader to two
/// (every package must be on exactly one of them). With `target_down` one more target is
/// configured on a port that refuses connections the whole run (M3R-12): it never takes
/// anything and must not keep the others from receiving or the stop from accounting.
pub fn stop_under_load_cell(
    test: &str,
    cell: Cell,
    target_down: bool,
    run_for: Duration,
    sig: &str,
) -> StopRun {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let real: Vec<String> = (0..cell.targets())
        .map(|i| ctx.key(&format!("t{}", i)))
        .collect();
    let mut clients: Vec<String> = real
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    if target_down {
        clients.push(ctx.client_yaml_at("down", "rmtest:down", closed_port()));
    }
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let t0 = real[0].clone();
    let done = Arc::new(AtomicBool::new(false));
    let producer = {
        let (url, src, done) = (ctx.ep.url.clone(), src.clone(), done.clone());
        thread::spawn(move || {
            let mut con = redis::Client::open(url.as_str())
                .unwrap()
                .get_connection()
                .unwrap();
            let mut seq = 0u64;
            while !done.load(Ordering::SeqCst) {
                let batch: Vec<String> = (seq..seq + 1000).map(seq_pkg).collect();
                let _: usize = con.rpush(&src, batch).unwrap();
                seq += 1000;
                // A backlog, not a flood: pause while the source holds 200000 packages
                while !done.load(Ordering::SeqCst) && con.llen::<_, usize>(&src).unwrap() > 200_000
                {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            seq
        })
    };
    ctx.wait_for("source loaded", |c| c.llen(&src) >= 10_000);
    let mut p = ctx.spawn(&cfg);
    // With ordering the first release comes after one buffer time
    ctx.wait_for("first package delivered", |c| c.llen(&t0) > 0);
    thread::sleep(run_for);
    p.stop_with(&ctx.pid_file(), sig);
    done.store(true, Ordering::SeqCst);
    let pushed = producer.join().unwrap();
    let mut delivered: Vec<u64> = Vec::new();
    for k in &real {
        let got: Vec<u64> = ctx.list(k).iter().map(|s| seq_of(s)).collect();
        if cell.in_order() {
            assert!(got.windows(2).all(|w| w[0] < w[1]), "{} not in order", k);
        }
        delivered.extend(got);
    }
    let left: Vec<u64> = ctx.list(&src).iter().map(|s| seq_of(s)).collect();
    assert_eq!(
        delivered.len() + left.len(),
        pushed as usize,
        "delivered {} + left {} != pushed {}",
        delivered.len(),
        left.len(),
        pushed
    );
    let taken = delivered.len() as u64;
    assert!(
        left.iter().copied().eq(taken..pushed),
        "source does not hold the newest packages in order"
    );
    let mut sorted = delivered.clone();
    sorted.sort_unstable();
    assert!(
        sorted.into_iter().eq(0..taken),
        "target does not hold every older package once"
    );
    StopRun { delivered, left }
}

/// A stand-in Redis target that is slow on purpose: each RPUSH waits `delay` before its
/// reply, and every pushed value is recorded in arrival order across all connections. A
/// test sets how long a batch takes to deliver, whatever the speed of the machine.
/// LLEN answers 0 (never over a limit) and is counted; any other command answers +OK.
/// close() takes it down for good: the port refuses connections and every open
/// connection is closed. A request is either recorded and answered before close() or not
/// recorded at all, so a test knows exactly what reached it.
pub struct SlowTarget {
    pub port: u16,
    shared: Arc<Mutex<SlowState>>,
}

#[derive(Default)]
struct SlowState {
    pushed: Vec<String>,
    client_delay: Duration,
    llens: usize,
    closed: bool,
    streams: Vec<TcpStream>,
    on_client: OnClient,
    after_push: AfterPush,
}

/// What a SlowTarget does after it records an RPUSH (M3R-20: a push the target may have
/// applied while the program never learns it)
#[derive(Clone, Copy, Default, PartialEq)]
pub enum AfterPush {
    #[default]
    Answer,
    /// Closes the connection instead of answering
    Close,
    /// Never answers: the program's command timeout ends the wait
    Silent,
}

/// What a SlowTarget does with a CLIENT command (CLIENT SETNAME at each connect)
#[derive(Clone, Copy, Default, PartialEq)]
pub enum OnClient {
    #[default]
    Ok,
    /// An error reply, as a server whose ACL denies the command
    Error,
    /// Closes the connection, as a proxy that does not allow the command (M3R-2)
    Close,
}

impl SlowTarget {
    pub fn start(delay: Duration) -> SlowTarget {
        SlowTarget::start_with(delay, OnClient::Ok)
    }

    pub fn start_with(delay: Duration, on_client: OnClient) -> SlowTarget {
        SlowTarget::start_on(0, delay, on_client, AfterPush::Answer)
    }

    /// A healthy target that answers late at start: CLIENT SETNAME, the first command of a
    /// connection, waits `late` before its reply (M3R-23)
    pub fn late(late: Duration) -> SlowTarget {
        let t = SlowTarget::start(Duration::ZERO);
        t.shared.lock().unwrap().client_delay = late;
        t
    }

    /// A target that records each RPUSH and then closes or stays silent (M3R-20)
    pub fn ambiguous(after_push: AfterPush) -> SlowTarget {
        SlowTarget::start_on(0, Duration::ZERO, OnClient::Ok, after_push)
    }

    /// On a given port (0: any), to bring a target up where one was refusing
    pub fn start_on(
        port: u16,
        delay: Duration,
        on_client: OnClient,
        after_push: AfterPush,
    ) -> SlowTarget {
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::new(Mutex::new(SlowState {
            on_client,
            after_push,
            ..SlowState::default()
        }));
        let st = shared.clone();
        // The accept thread ends at close() (dropping the listener closes the port) or with
        // the test process; each connection thread ends when its connection closes
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut g = st.lock().unwrap();
                if g.closed {
                    return;
                }
                g.streams.push(stream.try_clone().unwrap());
                drop(g);
                let st = st.clone();
                thread::spawn(move || slow_target_serve(stream, delay, st));
            }
        });
        SlowTarget { port, shared }
    }

    /// Values pushed so far, in arrival order
    pub fn pushed(&self) -> Vec<String> {
        self.shared.lock().unwrap().pushed.clone()
    }

    pub fn count(&self) -> usize {
        self.shared.lock().unwrap().pushed.len()
    }

    /// LLEN commands received so far
    pub fn llens(&self) -> usize {
        self.shared.lock().unwrap().llens
    }

    /// Take the target down: open connections are closed and the port refuses new ones
    pub fn close(&self) {
        let mut g = self.shared.lock().unwrap();
        g.closed = true;
        for s in g.streams.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        drop(g);
        // Wake the accept loop so it sees `closed` and drops the listener
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }

    /// The same target down and up again on its port, as a server restarted: every open
    /// connection closed, then a fresh target (nothing pushed yet) on the same port
    pub fn restart(&self) -> SlowTarget {
        self.close();
        // The accept loop drops the listener once it has seen `closed`
        let deadline = Instant::now() + Duration::from_secs(2);
        while TcpListener::bind(("127.0.0.1", self.port)).is_err() {
            assert!(Instant::now() < deadline, "port {} still bound", self.port);
            thread::sleep(Duration::from_millis(10));
        }
        SlowTarget::start_on(self.port, Duration::ZERO, OnClient::Ok, AfterPush::Answer)
    }
}

/// A stand-in target that accepts connections and reads what it is sent but never
/// answers: a server that hung, or a peer gone silent behind a firewall.
pub struct SilentTarget {
    pub port: u16,
}

impl SilentTarget {
    pub fn start() -> SilentTarget {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    while matches!(std::io::Read::read(&mut stream, &mut buf), Ok(n) if n > 0) {}
                });
            }
        });
        SilentTarget { port }
    }
}

/// A local port whose connections hang: its accept queue is full, so the kernel drops
/// new SYNs as a host behind a firewall that drops packets does (no refusal, no answer).
pub struct Blackhole {
    pub port: u16,
    _listener: TcpListener,
    _filler: TcpStream,
}

pub fn blackhole() -> Blackhole {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Linux: listen() again sets the backlog; with 0 one pending connection fills it
    // SAFETY: a valid listening socket owned by `listener`
    let rc = unsafe { libc::listen(std::os::fd::AsRawFd::as_raw_fd(&listener), 0) };
    assert_eq!(rc, 0, "listen");
    let filler = TcpStream::connect(("127.0.0.1", port)).unwrap();
    Blackhole {
        port,
        _listener: listener,
        _filler: filler,
    }
}

/// A local port where nothing listens (connections are refused)
pub fn closed_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// One RESP request (an array of bulk strings), or None at end of stream
fn resp_request(r: &mut impl BufRead) -> Option<Vec<Vec<u8>>> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let n: usize = line.trim_end().strip_prefix('*')?.parse().ok()?;
    let mut args = Vec::with_capacity(n);
    for _ in 0..n {
        line.clear();
        r.read_line(&mut line).ok()?;
        let len: usize = line.trim_end().strip_prefix('$')?.parse().ok()?;
        let mut buf = vec![0; len + 2];
        r.read_exact(&mut buf).ok()?;
        buf.truncate(len);
        args.push(buf);
    }
    Some(args)
}

fn slow_target_serve(stream: TcpStream, delay: Duration, st: Arc<Mutex<SlowState>>) {
    let mut w = stream.try_clone().unwrap();
    let mut r = BufReader::new(stream);
    while let Some(args) = resp_request(&mut r) {
        let cmd = args
            .first()
            .map(|c| String::from_utf8_lossy(c).to_ascii_uppercase())
            .unwrap_or_default();
        if cmd == "RPUSH" {
            thread::sleep(delay);
        }
        if cmd == "CLIENT" {
            let late = st.lock().unwrap().client_delay;
            thread::sleep(late);
        }
        // Recorded and answered under the lock, so close() falls before or after a request
        let mut g = st.lock().unwrap();
        if g.closed {
            return;
        }
        let reply = match cmd.as_str() {
            "RPUSH" => {
                g.pushed.extend(
                    args[2..]
                        .iter()
                        .map(|a| String::from_utf8_lossy(a).into_owned()),
                );
                match g.after_push {
                    AfterPush::Answer => format!(":{}\r\n", g.pushed.len()),
                    AfterPush::Close => {
                        let _ = w.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                    AfterPush::Silent => {
                        drop(g);
                        // Keep the connection open and never answer
                        loop {
                            thread::sleep(Duration::from_secs(3600));
                        }
                    }
                }
            }
            "LLEN" => {
                g.llens += 1;
                ":0\r\n".to_string()
            }
            "CLIENT" if g.on_client == OnClient::Close => {
                // A clone of the socket is kept for close(): shut it down for real
                let _ = w.shutdown(std::net::Shutdown::Both);
                return;
            }
            "CLIENT" if g.on_client == OnClient::Error => {
                "-NOPERM this user has no permissions to run the 'client' command\r\n".to_string()
            }
            _ => "+OK\r\n".to_string(),
        };
        if w.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}

/// One cell of the test matrix (project rule, M3R-12): ordering on or off, replicant or
/// spreader, one child or several.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ordering: bool,
    pub spreader: bool,
    pub children: u16,
}

impl Cell {
    pub fn mode(&self) -> &'static str {
        if self.spreader {
            "spreader"
        } else {
            "replicant"
        }
    }

    pub fn extra(&self) -> &'static str {
        if self.ordering {
            ORDERING_1S
        } else {
            ""
        }
    }

    /// Healthy targets a matrix test uses: one for replicant, two for spreader
    pub fn targets(&self) -> usize {
        if self.spreader {
            2
        } else {
            1
        }
    }

    /// Whether each target receives the packages in source order: with ordering (one
    /// sender, `ts` = push order) or with one child
    pub fn in_order(&self) -> bool {
        self.ordering || self.children == 1
    }
}

/// Eight #[test] functions, one per cell of the matrix, each calling `$body(name, cell)`
/// with a test name unique to the cell.
#[macro_export]
macro_rules! matrix {
    ($name:ident, $body:path, $several:expr) => {
        mod $name {
            use super::*;
            fn run(o: bool, s: bool, c: u16) {
                let tag = format!(
                    "{}_{}_{}_{}",
                    stringify!($name),
                    if o { "on" } else { "off" },
                    if s { "spr" } else { "rep" },
                    c
                );
                $body(
                    &tag,
                    Cell {
                        ordering: o,
                        spreader: s,
                        children: c,
                    },
                );
            }
            #[test]
            fn off_replicant_one() {
                run(false, false, 1)
            }
            #[test]
            fn off_replicant_several() {
                run(false, false, $several)
            }
            #[test]
            fn off_spreader_one() {
                run(false, true, 1)
            }
            #[test]
            fn off_spreader_several() {
                run(false, true, $several)
            }
            #[test]
            fn on_replicant_one() {
                run(true, false, 1)
            }
            #[test]
            fn on_replicant_several() {
                run(true, false, $several)
            }
            #[test]
            fn on_spreader_one() {
                run(true, true, 1)
            }
            #[test]
            fn on_spreader_several() {
                run(true, true, $several)
            }
        }
    };
}
