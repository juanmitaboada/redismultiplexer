//! Shared harness for the integration tests.
//!
//! Every test drives the real binary against the Redis server given by
//! RM_TEST_REDIS_URL (default redis://127.0.0.1:6379), using keys under
//! `rmtest:<test>:<unique suffix>:` that are deleted when the test ends.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use redis::Commands;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
        let out = self.dir.join("stdout.log");
        let err = self.dir.join("stderr.log");
        let child = Command::new(bin())
            .arg(config)
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
        // Shell builtin kill: no extra crate and no dependency on procps.
        let st = Command::new("sh")
            .args(["-c", "kill -INT \"$0\"", &self.pid().to_string()])
            .status()
            .unwrap();
        assert!(st.success(), "kill -INT {} failed", self.pid());
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
        self.sigint();
        let st = self.wait_exit();
        assert!(st.success(), "exit status after SIGINT: {:?}", st);
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
    p.stop(&ctx.pid_file());
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
