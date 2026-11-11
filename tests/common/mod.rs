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
    pub password: String,
    pub ssl: bool,
}

impl Endpoint {
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
        // The program always uses database 0 (it builds its URL without a path).
        assert!(
            path.is_empty() || path == "0",
            "{}: only database 0 is supported by redismultiplexer, got '/{}'",
            ENV_URL,
            path
        );
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (u, h),
            None => ("", authority),
        };
        let password = match userinfo.split_once(':') {
            Some((_, p)) => p.to_string(),
            None => String::new(),
        };
        // The program puts the password into a URL without percent-encoding it.
        assert!(
            !password.contains(['%', '@', '/', '#', '?', ':']),
            "{}: passwords with URL special characters are not supported by redismultiplexer",
            ENV_URL
        );
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
            password,
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
            "  - name: {}\n    ssl: {}\n    hostname: {}\n    port: {}\n    password: {}\n    channel: {}\n{}",
            q(name),
            self.ep.ssl,
            q(&self.ep.host),
            self.ep.port,
            q(&self.ep.password),
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
            "name: 'source'\nssl: {}\nhostname: {}\nport: {}\npassword: {}\nchannel: {}\nchildren: {}\nmode: {}\npid: {}\nstatus: {}\n{}clients:\n{}",
            self.ep.ssl,
            q(&self.ep.host),
            self.ep.port,
            q(&self.ep.password),
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

    /// As spawn, but with SIGHUP ignored when the program starts, as nohup leaves it.
    pub fn spawn_hup_ignored(&self, config: &Path) -> Proc {
        // `trap '' HUP` sets the ignore disposition, which exec keeps
        let script = "trap '' HUP; exec \"$0\" \"$1\"";
        self.spawn_cmd(Command::new("sh").args(["-c", script, bin()]).arg(config))
    }

    fn spawn_cmd(&self, cmd: &mut Command) -> Proc {
        let out = self.dir.join("stdout.log");
        let err = self.dir.join("stderr.log");
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

/// Ordering with a 1 s buffer on the `ts` field of `seq_pkg` packages.
pub const ORDERING_1S: &str =
    "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 1\nordering_limit: 200\n";

/// Package number `seq` of a stop-under-load run; the number is also its timestamp.
pub fn seq_pkg(seq: u64) -> String {
    format!("{{\"ts\": {}, \"v\": \"x\"}}", seq)
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
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(
        &src,
        children,
        "replicant",
        extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
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
    // With ordering the first release comes after one buffer time (whole seconds)
    ctx.wait_for("first package delivered", |c| c.llen(&t0) > 0);
    thread::sleep(run_for);
    p.stop_with(&ctx.pid_file(), sig);
    done.store(true, Ordering::SeqCst);
    let pushed = producer.join().unwrap();
    let delivered: Vec<u64> = ctx.list(&t0).iter().map(|s| seq_of(s)).collect();
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
/// LLEN answers 0 (never over a limit); any other command answers +OK.
pub struct SlowTarget {
    pub port: u16,
    pushed: Arc<Mutex<Vec<String>>>,
}

impl SlowTarget {
    pub fn start(delay: Duration) -> SlowTarget {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let pushed = Arc::new(Mutex::new(Vec::new()));
        let rec = pushed.clone();
        // The accept thread ends with the test process; each connection thread ends when
        // the program closes its connection
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let rec = rec.clone();
                thread::spawn(move || slow_target_serve(stream, delay, rec));
            }
        });
        SlowTarget { port, pushed }
    }

    /// Values pushed so far, in arrival order
    pub fn pushed(&self) -> Vec<String> {
        self.pushed.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.pushed.lock().unwrap().len()
    }
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

fn slow_target_serve(stream: TcpStream, delay: Duration, rec: Arc<Mutex<Vec<String>>>) {
    let mut w = stream.try_clone().unwrap();
    let mut r = BufReader::new(stream);
    while let Some(args) = resp_request(&mut r) {
        let cmd = args
            .first()
            .map(|c| String::from_utf8_lossy(c).to_ascii_uppercase())
            .unwrap_or_default();
        let reply = match cmd.as_str() {
            "RPUSH" => {
                thread::sleep(delay);
                let mut v = rec.lock().unwrap();
                v.extend(
                    args[2..]
                        .iter()
                        .map(|a| String::from_utf8_lossy(a).into_owned()),
                );
                format!(":{}\r\n", v.len())
            }
            "LLEN" => ":0\r\n".to_string(),
            _ => "+OK\r\n".to_string(),
        };
        if w.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}
