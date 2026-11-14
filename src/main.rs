//! RedisMultiplexer
//!
//! Author: Juanmi Taboada <juanmi@juanmitaboada.com>
//! Date: 28th June 2022
//!
//! This is a program to move packages between different Redis Queues using PUSH/POP
//! It does work between servers
//! It will keep control of your queues size by checking their length
//! It can filter data so it get to some queues or anothers and replace some small string in it
//! It can replicate data between queues or sparce it between them
//! It can reorder an incoming queue

// Lints present in 1.0.7, allowed so that CI can enforce clippy -D warnings on new code.
// Each one is tracked for removal in M3; unknown_lints keeps older clippy versions quiet.
#![allow(
    unknown_lints,
    clippy::absurd_extreme_comparisons,
    clippy::explicit_counter_loop,
    clippy::len_zero,
    clippy::needless_bool_assign,
    clippy::needless_borrow,
    clippy::needless_late_init,
    clippy::needless_return,
    clippy::partialeq_to_none,
    clippy::println_empty_string,
    clippy::ptr_arg,
    clippy::redundant_field_names,
    clippy::single_match,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unused_io_amount,
    clippy::useless_borrows_in_formatting,
    clippy::useless_conversion,
    clippy::useless_format
)]

// use std::mem;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::borrow::Cow;
use std::cmp;
use std::io::{self, stderr, stdout, Write};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::{cmp::Reverse, collections::BinaryHeap};

// use std::env;
use redis::{Commands, ConnectionLike};
use std::fs;
use std::path::{Path, PathBuf};

mod constants;
use constants::*;

mod debugger;
use debugger::*;

mod datetime;
use datetime::*;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ClientConfig {
    name: String,
    ssl: Option<bool>,
    hostname: String,
    port: u16,
    password: String,
    username: Option<String>,     // Redis 6 ACL user (M3-i)
    database: Option<u32>,        // SELECT on connect, 0 when absent (M1F-10)
    connect_timeout: Option<f64>, // seconds, 0 = the kernel's, as 1.0.8 (M3R-19)
    command_timeout: Option<f64>, // seconds, 0 = wait for ever, as 1.0.8 (M3R-19)
    channel: String,
    timelimit: Option<u64>,
    checklimit: Option<u64>,
    softlimit: Option<u64>,
    hardlimit: Option<u64>,
    deleteblock: Option<u64>,
    filter: Option<String>,
    filter_until: Option<String>,
    filter_until_included: Option<bool>,
    filter_limit: Option<usize>,
    filter_replace: Option<String>,
    filter_replace_header: Option<String>,
}

impl Clone for ClientConfig {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            ssl: self.ssl,
            hostname: self.hostname.clone(),
            port: self.port,
            password: self.password.clone(),
            username: self.username.clone(),
            database: self.database,
            connect_timeout: self.connect_timeout,
            command_timeout: self.command_timeout,
            channel: self.channel.clone(),
            timelimit: self.timelimit,
            checklimit: self.checklimit,
            softlimit: self.softlimit,
            hardlimit: self.hardlimit,
            deleteblock: self.deleteblock,
            filter: self.filter.clone(),
            filter_until: self.filter_until.clone(),
            filter_until_included: self.filter_until_included,
            filter_limit: self.filter_limit,
            filter_replace: self.filter_replace.clone(),
            filter_replace_header: self.filter_replace_header.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Config {
    name: String,
    ssl: Option<bool>,
    hostname: String,
    port: u16,
    password: String,
    username: Option<String>,     // Redis 6 ACL user (M3-i)
    database: Option<u32>,        // SELECT on connect, 0 when absent (M1F-10)
    connect_timeout: Option<f64>, // seconds, 0 = the kernel's, as 1.0.8 (M3R-19)
    command_timeout: Option<f64>, // seconds, 0 = wait for ever, as 1.0.8 (M3R-19)
    channel: String,
    children: u16,
    mode: String,
    pid: Option<String>,
    status: Option<String>,
    filter: Option<String>,
    filter_until: Option<String>,
    filter_until_included: Option<bool>,
    filter_limit: Option<usize>,
    filter_replace: Option<String>,
    filter_replace_header: Option<String>,
    ordering: Option<String>,
    ordering_buffer_time: Option<u64>,
    ordering_limit: Option<usize>,
    clients: Vec<ClientConfig>,
}

impl Clone for Config {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            ssl: self.ssl,
            hostname: self.hostname.clone(),
            port: self.port,
            password: self.password.clone(),
            username: self.username.clone(),
            database: self.database,
            connect_timeout: self.connect_timeout,
            command_timeout: self.command_timeout,
            channel: self.channel.clone(),
            children: self.children,
            mode: self.mode.clone(),
            pid: self.pid.clone(),
            status: self.status.clone(),
            filter: self.filter.clone(),
            filter_until: self.filter_until.clone(),
            filter_until_included: self.filter_until_included,
            filter_limit: self.filter_limit,
            filter_replace: self.filter_replace.clone(),
            filter_replace_header: self.filter_replace_header.clone(),
            ordering: self.ordering.clone(),
            ordering_buffer_time: self.ordering_buffer_time,
            ordering_limit: self.ordering_limit,
            clients: self.clients.clone(),
        }
    }
}

/// Keep track of clients we are connected to
struct RedisLink {
    config: ClientConfig,            // Client configuration
    link: Option<redis::Connection>, // None while the target cannot be reached (L3b)
    retry_at: Option<Instant>,       // no connect attempt before this
    backoff: Duration,               // wait after the next failure: 1 s, doubling to 5 s
    stuck_since: Option<Instant>,    // Some while the queue is stuck (hardlimit reached)
    packages: u64,                   // Packages we have seen from last check
    last_check: Option<Instant>,     // last check of the queue's size; None: never (L1a)
    filter: Option<Filter>,          // Client filter, compiled at start
    request: Vec<u8>,                // RPUSH request buffer, reused for every package
    endpoint: Endpoint,              // where to connect, built once
    connecting: Option<Connecting>,  // an attempt running outside this thread (X1c)
    down_since: Option<Instant>,     // when it was last seen working, while it is down
    attempts: u32,                   // failed connects since it went down (S4-4)
    skip_noted: bool,                // the "not connected yet" line was printed (M3R-23)
}

/// The result of a connect attempt that runs in its own short-lived thread, so a target
/// that does not answer never holds the thread that delivers (S4-3 = a, X1c)
type Connecting = Receiver<Result<redis::Connection, String>>;

/// Where and how to connect (M3-i): built from the configuration and handed to the crate as
/// is, never formatted into a URL, so a password reaches AUTH exactly as written (M1F-7)
/// and the database is selected (M1F-10)
#[derive(Clone, Debug, PartialEq)]
struct Endpoint {
    ssl: bool,
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    database: u32,
    connect_timeout: Option<Duration>, // None: no limit
    command_timeout: Option<Duration>, // None: no limit
    name: Option<String>,              // CLIENT SETNAME after the connect
}

/// The name a connection gets with CLIENT SETNAME, so operators and tests can tell them
/// apart: `redismultiplexer:<source>:<target>:<role>`; what a name may not hold (spaces,
/// control characters, anything outside printable ASCII) becomes `_`
fn connection_name(source: &str, target: &str, role: &str) -> String {
    format!("{}:{}:{}:{}", "redismultiplexer", source, target, role)
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .collect()
}

/// A timeout key as a Duration: absent is `default`, 0 is no limit (None). Checked by
/// verify_config, so a value that is not a duration cannot reach here.
fn timeout_key(value: Option<f64>, default: Duration) -> Option<Duration> {
    match value {
        None => Some(default),
        Some(0.0) => None,
        Some(v) => Duration::try_from_secs_f64(v).ok(),
    }
}

/// Error text when a timeout key is not 0 or a number of seconds a Duration holds, from
/// MIN_TIMEOUT up: a smaller one (1e-10) became a zero Duration, which the socket refuses,
/// and the program retried that for ever (M3R-27, D4d-5 = a)
fn check_timeout(owner: &str, key: &str, value: Option<f64>) -> Result<(), String> {
    match value {
        Some(v)
            if v != 0.0
                && (Duration::try_from_secs_f64(v).is_err() || v < MIN_TIMEOUT.as_secs_f64()) =>
        {
            Err(format!(
                "{} has an invalid {} {}: use 0 (no limit) or at least {} seconds",
                owner,
                key,
                v,
                MIN_TIMEOUT.as_secs_f64()
            ))
        }
        _ => Ok(()),
    }
}

/// An IPv6 address as written in a URL, `[::1]`, is the address `::1` for the resolver
/// (M3R-28: until 4a the URL parser took the brackets off; std's resolver refuses them)
fn unbracketed(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

impl Endpoint {
    #[allow(clippy::too_many_arguments)]
    fn new(
        ssl: Option<bool>,
        host: &str,
        port: u16,
        username: &Option<String>,
        password: &str,
        database: Option<u32>,
        connect_timeout: Option<f64>,
        command_timeout: Option<f64>,
    ) -> Endpoint {
        Endpoint {
            ssl: ssl.unwrap_or(false),
            host: unbracketed(host).to_string(),
            port,
            // Empty means none, as the URL path read an empty user or password: no AUTH
            username: username.clone().filter(|u| !u.is_empty()),
            password: Some(password.to_string()).filter(|p| !p.is_empty()),
            database: database.unwrap_or(0),
            connect_timeout: timeout_key(connect_timeout, DEFAULT_CONNECT_TIMEOUT),
            command_timeout: timeout_key(command_timeout, DEFAULT_COMMAND_TIMEOUT),
            name: None,
        }
    }

    fn named(self, name: String) -> Endpoint {
        Endpoint {
            name: Some(name),
            ..self
        }
    }

    fn source(config: &Config) -> Endpoint {
        Endpoint::new(
            config.ssl,
            &config.hostname,
            config.port,
            &config.username,
            &config.password,
            config.database,
            config.connect_timeout,
            config.command_timeout,
        )
    }

    fn target(config: &ClientConfig) -> Endpoint {
        Endpoint::new(
            config.ssl,
            &config.hostname,
            config.port,
            &config.username,
            &config.password,
            config.database,
            config.connect_timeout,
            config.command_timeout,
        )
    }

    fn info(&self) -> redis::ConnectionInfo {
        let addr = if self.ssl {
            redis::ConnectionAddr::TcpTls {
                host: self.host.clone(),
                port: self.port,
                insecure: false,
                tls_params: None,
            }
        } else {
            redis::ConnectionAddr::Tcp(self.host.clone(), self.port)
        };
        redis::ConnectionInfo {
            addr,
            redis: redis::RedisConnectionInfo {
                db: i64::from(self.database),
                username: self.username.clone(),
                password: self.password.clone(),
                ..Default::default()
            },
        }
    }
}

/// A compiled filter: a regex matched against the header of a package (its first `limit`
/// bytes, cut at the first `until`), with an optional replacement of that header.
/// Built once at start; each worker owns a clone, so no regex is cloned per package.
#[derive(Clone)]
struct Filter {
    regex: Regex,
    until: Option<memchr::memmem::Finder<'static>>,
    until_included: bool, // the delimiter belongs to the header (and to what is replaced)
    limit: usize,         // 0: no limit
    replace: Option<Replace>,
}

/// What a filter does to the header of a matching package
#[derive(Clone)]
enum Replace {
    Regex(Vec<u8>),  // filter_replace: first regex match replaced, $ groups expanded
    Header(Vec<u8>), // filter_replace_header: the whole header replaced by a literal
}

impl Filter {
    fn new(
        pattern: &str,
        until: Option<&str>,
        until_included: bool,
        limit: Option<usize>,
        replace: Option<Replace>,
    ) -> Result<Filter, String> {
        let regex = Regex::new(pattern).map_err(|e| e.to_string())?;
        Ok(Filter {
            regex,
            until: until
                .filter(|u| !u.is_empty())
                .map(|u| memchr::memmem::Finder::new(u.as_bytes()).into_owned()),
            until_included,
            limit: limit.unwrap_or(0),
            replace,
        })
    }

    /// None if the package does not match; otherwise the package to send, borrowed unless
    /// the replacement changed it
    fn apply<'a>(&self, data: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        // Limit counts bytes: it may cut a multibyte character, which is harmless
        let slice = if self.limit > 0 {
            &data[..cmp::min(self.limit, data.len())]
        } else {
            data
        };
        let haystack = match &self.until {
            Some(f) => match f.find(slice) {
                Some(idx) if self.until_included => &slice[..idx + f.needle().len()],
                Some(idx) => &slice[..idx],
                None => slice,
            },
            None => slice,
        };
        if !self.regex.is_match(haystack) {
            return None;
        }
        let tail = &data[haystack.len()..];
        match &self.replace {
            None => Some(Cow::Borrowed(data)),
            Some(Replace::Regex(r)) => {
                let replaced = self.regex.replace(haystack, r.as_slice());
                Some(Cow::Owned([&replaced[..], tail].concat()))
            }
            Some(Replace::Header(h)) => Some(Cow::Owned([h.as_slice(), tail].concat())),
        }
    }
}

/// The root filter and one optional filter per client, in configuration order
#[derive(Clone)]
struct Filters {
    root: Option<Filter>,
    clients: Vec<Option<Filter>>,
}

/// The replacement a filter applies, from its two optional keys (verify_config rejects both)
fn replacement(replace: &Option<String>, replace_header: &Option<String>) -> Option<Replace> {
    match (replace, replace_header) {
        (Some(r), _) => Some(Replace::Regex(r.as_bytes().to_vec())),
        (None, Some(h)) => Some(Replace::Header(h.as_bytes().to_vec())),
        (None, None) => None,
    }
}

/// Compile every filter of a configuration
fn build_filters(config: &Config) -> Result<Filters, String> {
    let root = match &config.filter {
        Some(f) => Some(
            Filter::new(
                f,
                config.filter_until.as_deref(),
                config.filter_until_included.unwrap_or(false),
                config.filter_limit,
                replacement(&config.filter_replace, &config.filter_replace_header),
            )
            .map_err(|e| format!("Filter Regex '{}' doesn't compile: {}", f, e))?,
        ),
        None => None,
    };
    let mut clients = Vec::new();
    for client in &config.clients {
        clients.push(match &client.filter {
            Some(f) => Some(
                Filter::new(
                    f,
                    client.filter_until.as_deref(),
                    client.filter_until_included.unwrap_or(false),
                    client.filter_limit,
                    replacement(&client.filter_replace, &client.filter_replace_header),
                )
                .map_err(|e| {
                    format!(
                        "Client '{}' has a filter Regex '{}' that doesn't compile: {}",
                        client.name, f, e
                    )
                })?,
            ),
            None => None,
        });
    }
    Ok(Filters { root, clients })
}

/// What a thread counts between two reports, and what main sums. `dropped` is the
/// packages no target took; `filtered + stuck + failed` is the same packages by why
/// (M3-e = ea, D4d-3 = a)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Counts {
    incoming: u64,
    outgoing: u64,
    dropped: u64,
    deleted: u64,
    filtered: u64,
    stuck: u64,
    failed: u64,
}

impl Counts {
    fn add(&mut self, other: &Counts) {
        self.incoming += other.incoming;
        self.outgoing += other.outgoing;
        self.dropped += other.dropped;
        self.deleted += other.deleted;
        self.filtered += other.filtered;
        self.stuck += other.stuck;
        self.failed += other.failed;
    }
}

/// Counters of one worker, written by that worker only and read by main. Relaxed atomics:
/// every counter stands alone and main only needs a recent value. Aligned to a cache line,
/// so two workers never write the same line.
#[repr(align(64))]
struct WorkerStats {
    incoming: AtomicU64,
    outgoing: AtomicU64,
    dropped: AtomicU64,
    deleted: AtomicU64,
    filtered: AtomicU64,
    stuck: AtomicU64,
    failed: AtomicU64,
    stuck_targets: Vec<AtomicBool>, // per client, in configuration order
    down_targets: Vec<AtomicBool>,  // per client: no link at the last report
}

impl WorkerStats {
    fn new(clients: usize) -> WorkerStats {
        WorkerStats {
            incoming: AtomicU64::new(0),
            outgoing: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            deleted: AtomicU64::new(0),
            filtered: AtomicU64::new(0),
            stuck: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            stuck_targets: (0..clients).map(|_| AtomicBool::new(false)).collect(),
            down_targets: (0..clients).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Add the counts a worker gathered since its last report
    fn add(&self, c: &Counts) {
        self.incoming.fetch_add(c.incoming, Ordering::Relaxed);
        self.outgoing.fetch_add(c.outgoing, Ordering::Relaxed);
        self.dropped.fetch_add(c.dropped, Ordering::Relaxed);
        self.deleted.fetch_add(c.deleted, Ordering::Relaxed);
        self.filtered.fetch_add(c.filtered, Ordering::Relaxed);
        self.stuck.fetch_add(c.stuck, Ordering::Relaxed);
        self.failed.fetch_add(c.failed, Ordering::Relaxed);
    }
}

/// One counter slot per worker
fn stats_slots(workers: u16, clients: usize) -> Arc<Vec<WorkerStats>> {
    Arc::new((0..workers).map(|_| WorkerStats::new(clients)).collect())
}

/// The slot worker `id` writes: it must be its own, or main reports one worker's counts and
/// stuck targets as another's
fn worker_slot(stats: &[WorkerStats], id: u16) -> &WorkerStats {
    &stats[id as usize]
}

/// Counts of every worker since the last call (each slot is reset)
fn take_counts(stats: &[WorkerStats]) -> Counts {
    let mut t = Counts::default();
    for w in stats {
        t.add(&Counts {
            incoming: w.incoming.swap(0, Ordering::Relaxed),
            outgoing: w.outgoing.swap(0, Ordering::Relaxed),
            dropped: w.dropped.swap(0, Ordering::Relaxed),
            deleted: w.deleted.swap(0, Ordering::Relaxed),
            filtered: w.filtered.swap(0, Ordering::Relaxed),
            stuck: w.stuck.swap(0, Ordering::Relaxed),
            failed: w.failed.swap(0, Ordering::Relaxed),
        });
    }
    t
}

/// Indexes of the clients that some worker found stuck at its last report
fn stuck_clients(stats: &[WorkerStats], clients: usize) -> Vec<usize> {
    (0..clients)
        .filter(|&c| {
            stats
                .iter()
                .any(|w| w.stuck_targets[c].load(Ordering::Relaxed))
        })
        .collect()
}

/// Indexes of the clients that some worker had without a link at its last report
fn down_clients(stats: &[WorkerStats], clients: usize) -> Vec<usize> {
    (0..clients)
        .filter(|&c| {
            stats
                .iter()
                .any(|w| w.down_targets[c].load(Ordering::Relaxed))
        })
        .collect()
}

/// Exit status (M3-h = ha): 0 after a stop by signal, `--version` and `--help`; 2 for a
/// usage or configuration error; 1 when a thread ended on its own (S4-5 = a)
const EXIT_OK: i32 = 0;
const EXIT_FAILED: i32 = 1;
const EXIT_CONFIG: i32 = 2;

/// How a run ended
#[derive(Debug, PartialEq)]
struct Ending {
    /// A worker or the sender ended on its own (a panic): the program stopped for it
    died: bool,
    /// Packages the ordering buffer still held when its sender was gone: lost
    abandoned: u64,
}

/// Main's watch over the threads: waits for a signal, a second at a time (calling `tick`
/// each time), or for a worker or the sender to end on its own (S4-5 = a). Then the stop:
/// the workers are told and joined; the sender, if it is alive, flushes the buffer and is
/// joined; if it died, what the buffer holds is counted as abandoned. A thread that
/// panicked during the stop counts as died too. Never waits for a thread that is gone.
fn supervise(
    stop: &AtomicBool,
    signals: &Receiver<bool>,
    workers: Vec<thread::JoinHandle<()>>,
    sender: Option<(Arc<OrderBuffer>, thread::JoinHandle<()>)>,
    mut tick: impl FnMut(),
) -> Ending {
    let mut died = false;
    loop {
        // Wait up to a second for a signal (instead of sleeping, so a stop is seen at once)
        if signals.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
        // A thread only ends when told to: one that ended on its own stops the program
        if workers.iter().any(|h| h.is_finished()) {
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_YELLOW,
                0,
                "Some child closed abruptly, closing everything!"
            );
            died = true;
            break;
        }
        if sender.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_YELLOW,
                0,
                "The ordered sender closed abruptly, closing everything!"
            );
            died = true;
            break;
        }
        tick();
    }

    // Tell all children to finish their job
    stop.store(true, Ordering::Relaxed);

    // Wait for the children: each leaves after its current package (a BLPOP waits at most
    // 1 s); say how many are left every second
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Closing...");
    let mut said = Instant::now();
    while !workers.iter().all(|h| h.is_finished()) {
        if said.elapsed() >= Duration::from_secs(1) {
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_YELLOW,
                0,
                "There are {} threads left!",
                workers.iter().filter(|h| !h.is_finished()).count()
            );
            said = Instant::now();
        }
        thread::sleep(Duration::from_millis(100));
    }
    for handle in workers {
        died |= handle.join().is_err();
    }

    // Every child has left, so nothing more enters the ordering buffer: the sender flushes
    // it in ts order and ends. A sender that is gone leaves what the buffer holds.
    let mut abandoned = 0;
    if let Some((buffer, handle)) = sender {
        buffer.finish();
        if handle.join().is_err() {
            died = true;
            abandoned = buffer.abandon();
        }
    }
    Ending { died, abandoned }
}

/// Main module will manage the basics from this program
fn main() {
    let code = run();
    // process::exit runs no destructors: everything was dropped inside run() (the pid file
    // removed); only stdout may hold a partial line
    let _ = stdout().flush();
    process::exit(code);
}

/// The program; its exit status
fn run() -> i32 {
    // Get args
    let args: Vec<String> = std::env::args().collect();

    // Check if we got instructed with the path to configuration
    if args.len() > 1 {
        // Check if version is requested
        if (args[1] == "version") || (args[1] == "--version") {
            // Answer with our version
            println!("{} v{}", PROGRAM_NAME, VERSION);
            EXIT_OK
        } else if (args[1] == "help") || (args[1] == "--help") || (args[1] == "-h") {
            // Welcome
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_CYAN,
                COLOR_NOHEAD,
                "{} v{}",
                PROGRAM_NAME,
                VERSION
            );

            // Missing argument, show some help
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
            print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_YELLOW,
                COLOR_NOHEAD,
                "Usage: redismultiplexer <path_to_config.yaml> [debug]"
            );
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
            print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_WHITE,
                COLOR_NOHEAD,
                "Documentation at: https://github.com/juanmitaboada/redismultiplexer"
            );
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");

            // Answer with out version
            println!("");
            EXIT_OK
        } else {
            // Welcome
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_CYAN,
                0,
                "{} v{}",
                PROGRAM_NAME,
                VERSION
            );

            #[cfg(feature = "debug")]
            print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "Debug is enabled!");

            // Is a systemd call
            let systemd = (args.len() == 3) && (args[2] == "systemd");

            // Build path to configuration
            let final_path: String;
            if systemd {
                final_path = format!("/etc/redismultiplexer/{}.yaml", &args[1]);
            } else {
                final_path = args[1].clone();
            }

            // A configuration that cannot be used ends the program with EXIT_CONFIG
            let mut code = EXIT_CONFIG;

            // Read config
            let mut config: Option<Config> = None;
            match get_config(final_path) {
                Ok(c) => config = Some(c),
                Err(e) => print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_RED,
                    0,
                    "Error while processing configuration: {}",
                    e
                ),
            }

            // Check if we can keep working
            if let Some(inconfig) = config {
                let mut error = false;

                // If in systemd call, overwrite pid and status
                let pidfile: Option<String>;
                let statusfile: Option<String>;
                if systemd {
                    pidfile = Some(format!("/run/redismultiplexer/{}.pid", &args[1]));
                    statusfile = Some(format!("/run/redismultiplexer/{}.status", &args[1]));
                } else {
                    // Get path to pid
                    if let Some(pid) = inconfig.pid.clone() {
                        pidfile = Some(pid);
                    } else {
                        pidfile = None;
                    }

                    // Get path to status
                    if let Some(stat) = inconfig.status.clone() {
                        statusfile = Some(stat);
                    } else {
                        statusfile = None;
                    }
                }

                // Write our pid
                if let Some(pid) = &pidfile {
                    match fs::write(pid, format!("{}", process::id())) {
                        Ok(_) => (),
                        Err(e) => {
                            print_debug!(
                                PROGRAM_NAME,
                                stderr(),
                                COLOR_RED,
                                0,
                                "Unable to write PID at {}: {}",
                                pid,
                                e
                            );
                            error = true;
                        }
                    }
                }

                // If not error until here
                if !error {
                    // Compile filters once (verify_config already checked that they compile)
                    let filters = build_filters(&inconfig).expect("filters checked at start");

                    // Main ordering regex (verify_config already checked that it compiles)
                    let ordering_regex: Option<Regex> = inconfig
                        .ordering
                        .as_ref()
                        .map(|r| Regex::new(r).expect("ordering regex checked at start"));

                    if !error {
                        // Stop order, read by every child after each package. SIGINT, SIGTERM
                        // and SIGHUP set it (ctrlc `termination`, M3-l = la) and wake this loop.
                        let stop = Arc::new(AtomicBool::new(false));
                        let (keepworking_tx, keepworking_rx): (Sender<bool>, Receiver<bool>) =
                            mpsc::channel();
                        let handler_stop = stop.clone();
                        // Started under nohup (SIGHUP ignored): keep ignoring it, as before
                        // ctrlc took SIGHUP too (M3F-14)
                        let hup_ignored = sighup_ignored();
                        ctrlc::set_handler(move || {
                            print_debug!(
                                PROGRAM_NAME,
                                stdout(),
                                COLOR_GREEN,
                                0,
                                "User requested to exit!"
                            );
                            handler_stop.store(true, Ordering::Relaxed);
                            // Main may be past its loop already (a second signal)
                            let _ = keepworking_tx.send(false);
                        })
                        .expect("Error setting Ctrl-C handler");
                        if hup_ignored {
                            ignore_sighup();
                        }

                        // Ordering: children put timestamped packages in one buffer and one
                        // sender delivers them from it, so the target order is strict (M3-k)
                        let ordered: Option<Arc<OrderBuffer>> = ordering_regex
                            .as_ref()
                            .map(|_| Arc::new(OrderBuffer::new()));
                        let hold = Duration::from_secs(inconfig.ordering_buffer_time.unwrap_or(0));

                        // Counters of every child, and of the sender after them (no messages)
                        let stats = stats_slots(
                            inconfig.children + ordered.is_some() as u16,
                            inconfig.clients.len(),
                        );

                        // The sender: its own target connections and filters, its stats slot
                        let sender_handle = ordered.as_ref().map(|buffer| {
                            let buffer = buffer.clone();
                            let sender_config = inconfig.clone();
                            let fr = filters.clone();
                            let sender_stats = stats.clone();
                            let id = inconfig.children;
                            thread::spawn(move || {
                                sender(id, &buffer, hold, sender_config, fr, sender_stats)
                            })
                        });

                        // Prepare list of handles
                        let mut handles: Vec<thread::JoinHandle<_>> = Vec::new();

                        // Spawn a number of threads and collect their join handles
                        for id in 0..inconfig.children {
                            // Clone config and execute thread
                            let child_stats = stats.clone();
                            let child_stop = stop.clone();
                            let child_ordered = ordered.clone();
                            let child_config = inconfig.clone();
                            // Each child owns its compiled filters (one regex cache per thread)
                            let fr = filters.clone();
                            let or = ordering_regex.clone();
                            let handle = thread::spawn(move || {
                                child(
                                    id,
                                    or,
                                    inconfig.ordering_limit,
                                    child_stats,
                                    child_stop,
                                    child_ordered,
                                    child_config,
                                    fr,
                                );
                            });
                            handles.push(handle);
                        }

                        // Show configuration
                        print_debug!(
                            PROGRAM_NAME,
                            stdout(),
                            COLOR_CYAN,
                            0,
                            "> {}:{} @ {} [{}, {} children]",
                            inconfig.hostname,
                            inconfig.port,
                            inconfig.channel,
                            inconfig.mode,
                            inconfig.children
                        );
                        for client in &inconfig.clients {
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "  - {}:{} @ {}  [timelimit={}, checklimit={}, softlimit={}, hardlimit={}]", client.hostname, client.port, client.channel, option2string!(client.timelimit), option2string!(client.checklimit), option2string!(client.softlimit), option2string!(client.hardlimit));
                        }

                        // Keep working until a signal, or until a thread ends on its own
                        let mut lasttime = get_current_time_with_ms();
                        let mut since_start = Counts::default();
                        let sender = ordered.clone().zip(sender_handle);
                        let ending = supervise(&stop, &keepworking_rx, handles, sender, || {
                            // Check if we should show statistics
                            if (get_current_time_with_ms() - (STATISTICS_SECONDS * 1000)) > lasttime
                            {
                                statistics(
                                    &inconfig.name,
                                    ordered.as_deref(),
                                    &inconfig.clients,
                                    &stats,
                                    &statusfile,
                                    lasttime,
                                    &mut since_start,
                                );
                                lasttime = get_current_time_with_ms();
                            }
                        });

                        // What the buffer held when its sender died is lost: dropped, counted
                        // in the sender's slot so it reaches the status file (S4-5 = a)
                        if ending.abandoned > 0 {
                            worker_slot(&stats, inconfig.children).add(&Counts {
                                dropped: ending.abandoned,
                                failed: ending.abandoned,
                                ..Counts::default()
                            });
                        }

                        // What was counted since the last line, the flush and any drop at the
                        // stop included, still reaches the status file (M3R-12)
                        statistics(
                            &inconfig.name,
                            ordered.as_deref(),
                            &inconfig.clients,
                            &stats,
                            &statusfile,
                            lasttime,
                            &mut since_start,
                        );

                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "Program finished!");
                        code = if ending.died { EXIT_FAILED } else { EXIT_OK };
                    }

                    // Remove pid
                    if let Some(pid) = &pidfile {
                        fs::remove_file(pid).unwrap_or_else(|why| {
                            print_debug!(
                                PROGRAM_NAME,
                                stderr(),
                                COLOR_YELLOW,
                                0,
                                "Couldn't delete PID at {}: {}",
                                pid,
                                why
                            ); // why.kind() shows the basic error
                        });
                    }
                }
            }
            code
        }
    } else {
        // Welcome
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_CYAN,
            COLOR_NOHEAD,
            "{} v{}",
            PROGRAM_NAME,
            VERSION
        );

        // Missing argument, show some help
        print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
        print_debug!(
            PROGRAM_NAME,
            stderr(),
            COLOR_YELLOW,
            COLOR_NOHEAD,
            "Usage: redismultiplexer <path_to_config.yaml> [debug]"
        );
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "");
        print_debug!(
            PROGRAM_NAME,
            stderr(),
            COLOR_WHITE,
            COLOR_NOHEAD,
            "Documentation at: https://github.com/juanmitaboada/redismultiplexer"
        );
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "");
        print_debug!(
            PROGRAM_NAME,
            stderr(),
            COLOR_CYAN,
            COLOR_NOHEAD,
            "Systemd: redismultiplexer <config_prefix> start"
        );
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "    <config_prefix>: is the configuration at /etc/redismultiplexer/<config_prefix>.yaml");
        print_debug!(
            PROGRAM_NAME,
            stderr(),
            COLOR_WHITE,
            COLOR_NOHEAD,
            "    pid configuration entry will be set as /run/redismultiplexer/<config_prefix>.pid"
        );
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "    status configuration entry will be set as /run/redismultiplexer/<config_prefix>.status");
        EXIT_CONFIG
    }
}

/// Do basic checks on configuration
fn verify_config(config: Config) -> Result<Config, String> {
    // To keep sense on source code
    let source = &config;

    // Verify working mode
    if (config.mode != "replicant") && (config.mode != "spreader") {
        return Err(format!(
            "Mode '{}' is unknown, valid modes are: replicant and spreader",
            config.mode
        ));
    }

    // Verify children
    if config.children <= 0 {
        return Err(format!(
            "Children is set to '{}', there is nothing to do without children",
            config.children
        ));
    }

    // Verify source name
    if source.name.len() == 0 {
        return Err(format!(
            "Source '{}' has an empty name [hostname=\"{}\", port={}, channel=\"{}\"]",
            source.name, source.hostname, source.port, source.channel
        ));
    }

    // Verify source hostname
    if source.hostname.len() == 0 {
        return Err(format!(
            "Source '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]",
            source.name, source.hostname, source.port, source.channel
        ));
    }

    // Verify source channel
    if source.channel.len() == 0 {
        return Err(format!(
            "Source '{}' has an empty channel [hostname=\"{}\", port={}, channel=\"{}\"]",
            source.name, source.hostname, source.port, source.channel
        ));
    }

    // Timeouts: 0 or a positive number of seconds
    let owner = format!("Source '{}'", source.name);
    check_timeout(&owner, "connect_timeout", source.connect_timeout)?;
    check_timeout(&owner, "command_timeout", source.command_timeout)?;

    // === FILTERS ===

    // Filter
    match &source.filter {
        None => {
            let keys = filter_options_set(
                &source.filter_until,
                source.filter_until_included,
                source.filter_limit,
                &source.filter_replace,
                &source.filter_replace_header,
            );
            if !keys.is_empty() {
                return Err(format!(
                    "Source '{}' sets {} without filter: the filtering options need a filter",
                    source.name,
                    keys.join(", ")
                ));
            }
        }
        Some(v) => {
            if v.len() == 0 {
                return Err(format!(
                    "Source '{}' is using filters, but filter can not empty",
                    source.name
                ));
            }
            if source.filter_replace.is_some() && source.filter_replace_header.is_some() {
                return Err(format!(
                    "Source '{}' sets both filter_replace and filter_replace_header, use only one",
                    source.name
                ));
            }
        }
    }

    // === ORDERING ===

    // No combination of the three keys is an error (M3-m = ma, M1F-5, D4d-2 = a): the odd
    // ones get a warning at start (config_warnings). The regex must compile (main checked
    // it before, after the pid file was written).
    if let Some(r) = &source.ordering {
        if let Err(e) = Regex::new(r) {
            return Err(format!(
                "Source '{}' has an ordering Regex '{}' that doesn't compile: {}",
                source.name, r, e
            ));
        }
    }

    // Verify there are clients
    if config.clients.len() > 0 {
        // Verify all clients
        for client in &config.clients {
            // Verify that hostname is not empty (an empty name only gets a warning, M1F-6)
            if client.hostname.len() == 0 {
                return Err(format!(
                    "Client '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]",
                    client.name, client.hostname, client.port, client.channel
                ));
            }

            // Verify that channel is not empty
            if client.channel.len() == 0 {
                return Err(format!(
                    "Client '{}' has an empty channel [hostname=\"{}\", port={}, channel=\"{}\"]",
                    client.name, client.hostname, client.port, client.channel
                ));
            }

            // Timeouts: 0 or a positive number of seconds
            let owner = format!("Client '{}'", client.name);
            check_timeout(&owner, "connect_timeout", client.connect_timeout)?;
            check_timeout(&owner, "command_timeout", client.command_timeout)?;

            // Verify that source and target are not the same
            if (source.hostname == client.hostname)
                && (source.port == client.port)
                && (source.channel == client.channel)
            {
                return Err(format!("Client '{}' is using same connection information than source [hostname=\"{}\", port={}, channel=\"{}\"]",client.name, client.hostname, client.port, client.channel));
            }

            // === LIMITS ===

            // A hardlimit turns the limits on, the other keys have defaults (M3-f = fa); any
            // of them without a hardlimit is an error (they would do nothing). 0 is not set.
            if client.hardlimit.unwrap_or(0) == 0 {
                for (key, value) in [
                    ("timelimit", client.timelimit),
                    ("checklimit", client.checklimit),
                    ("softlimit", client.softlimit),
                ] {
                    if value.unwrap_or(0) > 0 {
                        return Err(format!("Client '{}' sets {} without hardlimit: the limits need a hardlimit bigger than 0", client.name, key));
                    }
                }
            }

            // === FILTERS ===

            // Filter
            match &client.filter {
                None => {
                    let keys = filter_options_set(
                        &client.filter_until,
                        client.filter_until_included,
                        client.filter_limit,
                        &client.filter_replace,
                        &client.filter_replace_header,
                    );
                    if !keys.is_empty() {
                        return Err(format!(
                            "Client '{}' sets {} without filter: the filtering options need a filter",
                            client.name,
                            keys.join(", ")
                        ));
                    }
                }
                Some(v) => {
                    if v.len() == 0 {
                        return Err(format!(
                            "Client '{}' is using filters, but filter can not empty",
                            client.name
                        ));
                    }
                    if client.filter_replace.is_some() && client.filter_replace_header.is_some() {
                        return Err(format!(
                            "Client '{}' sets both filter_replace and filter_replace_header, use only one",
                            client.name
                        ));
                    }
                }
            }
        }
    } else {
        return Err(format!(
            "No clients found, you need clients to make this to work"
        ));
    }

    // Every filter must compile: a bad client regex used to panic the workers (M1F-8)
    build_filters(&config)?;

    let mut config = config;
    for client in config.clients.iter_mut() {
        zero_limits_are_absent(client);
        // `deleteblock: 0` is not set either (C4c-1 = a): the target is stuck at hardlimit
        // instead of the endless LTRIM loop of 1.0.8 (M3F-4); a warning says so
        if client.deleteblock == Some(0) {
            client.deleteblock = None;
        }
        limits_defaults(client);
    }
    return Ok(config);
}

/// The filtering options set, by key name, for the error when `filter` is missing (M3R-10)
fn filter_options_set(
    until: &Option<String>,
    until_included: Option<bool>,
    limit: Option<usize>,
    replace: &Option<String>,
    replace_header: &Option<String>,
) -> Vec<&'static str> {
    [
        ("filter_until", until.is_some()),
        ("filter_until_included", until_included.is_some()),
        ("filter_limit", limit.is_some()),
        ("filter_replace", replace.is_some()),
        ("filter_replace_header", replace_header.is_some()),
    ]
    .into_iter()
    .filter(|(_, set)| *set)
    .map(|(key, _)| key)
    .collect()
}

/// Odd but working settings: the program starts and says so (M3-m = ma). Read on the
/// configuration as parsed, before verify_config reads 0 as not set.
fn config_warnings(config: &Config) -> Vec<String> {
    let mut warnings = Vec::new();
    let source = format!("Source '{}'", config.name);
    // Ordering (D4d-2 = a): every combination starts
    let ordering_keys: Vec<&str> = [
        (
            "ordering_buffer_time",
            config.ordering_buffer_time.is_some(),
        ),
        ("ordering_limit", config.ordering_limit.is_some()),
    ]
    .into_iter()
    .filter(|(_, set)| *set)
    .map(|(key, _)| key)
    .collect();
    match config.ordering.as_deref() {
        None => {
            if !ordering_keys.is_empty() {
                warnings.push(format!(
                    "{} sets {} without ordering: ignored",
                    source,
                    ordering_keys.join(", ")
                ));
            }
        }
        Some("") => warnings.push(format!(
            "{} has an empty ordering: every package is delivered at once, without ordering information",
            source
        )),
        Some(_) => {
            if config.ordering_buffer_time.unwrap_or(0) == 0 {
                warnings.push(format!(
                    "{} uses ordering without ordering_buffer_time: packages are released at once, still in order",
                    source
                ));
            }
        }
    }
    let mut names: Vec<&str> = Vec::new();
    for client in &config.clients {
        let target = format!("Client '{}'", client.name);
        // An empty name works, it only makes the log harder to read (M1F-6)
        if client.name.is_empty() {
            warnings.push(format!(
                "A client has an empty name [hostname=\"{}\", port={}, channel=\"{}\"]",
                client.hostname, client.port, client.channel
            ));
        } else if names.contains(&client.name.as_str()) {
            warnings.push(format!(
                "{} is the name of more than one client: the log and the status file cannot tell them apart",
                target
            ));
        } else {
            names.push(&client.name);
        }
        if let (Some(soft), Some(hard)) = (client.softlimit, client.hardlimit) {
            if soft > 0 && hard > 0 && soft >= hard {
                warnings.push(format!(
                    "{} has softlimit {} at or above hardlimit {}: the target is freed at the check right after it gets stuck",
                    target, soft, hard
                ));
            }
        }
        if client.deleteblock == Some(0) {
            warnings.push(format!(
                "{} has deleteblock: 0, read as not set: the target is stuck at hardlimit instead of trimmed",
                target
            ));
        }
    }
    warnings
}

/// The keys a configuration file may have, at the root and in a client
const ROOT_KEYS: &[&str] = &[
    "name",
    "ssl",
    "hostname",
    "port",
    "password",
    "username",
    "database",
    "connect_timeout",
    "command_timeout",
    "channel",
    "children",
    "mode",
    "pid",
    "status",
    "filter",
    "filter_until",
    "filter_until_included",
    "filter_limit",
    "filter_replace",
    "filter_replace_header",
    "ordering",
    "ordering_buffer_time",
    "ordering_limit",
    "clients",
];
const CLIENT_KEYS: &[&str] = &[
    "name",
    "ssl",
    "hostname",
    "port",
    "password",
    "username",
    "database",
    "connect_timeout",
    "command_timeout",
    "channel",
    "timelimit",
    "checklimit",
    "softlimit",
    "hardlimit",
    "deleteblock",
    "filter",
    "filter_until",
    "filter_until_included",
    "filter_limit",
    "filter_replace",
    "filter_replace_header",
];

/// Keys the program does not know (a typo such as `filter_untill` is silently no filter),
/// from a second, untyped read of the text. Not through `#[serde(flatten)]` on the typed
/// read: that buffers the values, and an unquoted number on a text key (`password: 1234`,
/// which 1.0.8 reads) would then fail to read as text. If the untyped read fails, no
/// warnings: the typed read already decided whether the file is used.
fn unknown_keys(data: &str) -> Vec<String> {
    let Ok(serde_norway::Value::Mapping(root)) = serde_norway::from_str(data) else {
        return Vec::new();
    };
    let unknown = |map: &serde_norway::Mapping, known: &[&str]| -> Vec<String> {
        map.keys()
            .map(|k| match k {
                serde_norway::Value::String(s) => s.clone(),
                other => format!("{:?}", other),
            })
            .filter(|k| !known.contains(&k.as_str()))
            .collect()
    };
    let mut warnings: Vec<String> = unknown(&root, ROOT_KEYS)
        .into_iter()
        .map(|k| {
            format!(
                "Unknown key '{}' at the root of the configuration: ignored",
                k
            )
        })
        .collect();
    if let Some(serde_norway::Value::Sequence(clients)) = root.get("clients") {
        for (n, client) in clients.iter().enumerate() {
            if let serde_norway::Value::Mapping(map) = client {
                let name = match map.get("name") {
                    Some(serde_norway::Value::String(s)) => s.clone(),
                    _ => format!("#{}", n + 1),
                };
                for k in unknown(map, CLIENT_KEYS) {
                    warnings.push(format!("Unknown key '{}' in client '{}': ignored", k, name));
                }
            }
        }
    }
    warnings
}

/// A target with a hardlimit and some of the other limit keys absent (M3-f = fa): softlimit
/// 80 % of hardlimit, at least 1 so it can be freed (L4c-2); timelimit 1 s and checklimit
/// 10000, each when not given, so a check comes at whichever is first (L4c-5)
fn limits_defaults(client: &mut ClientConfig) {
    if let Some(hard) = client.hardlimit {
        client
            .softlimit
            // hard * 4 / 5 without overflowing a u64
            .get_or_insert(cmp::max(1, hard / 5 * 4 + hard % 5 * 4 / 5));
        client.timelimit.get_or_insert(DEFAULT_TIMELIMIT);
        client.checklimit.get_or_insert(DEFAULT_CHECKLIMIT);
    }
}

/// A limit key at 0 is not set (L4c-1): the check above always read it so, and now the
/// delivery does too (a target with every limit at 0 was stuck for the whole run and
/// dropped every package, M3F-17; `checklimit: 0` alone cost an LLEN per package)
fn zero_limits_are_absent(client: &mut ClientConfig) {
    for key in [
        &mut client.timelimit,
        &mut client.checklimit,
        &mut client.softlimit,
        &mut client.hardlimit,
    ] {
        if *key == Some(0) {
            *key = None;
        }
    }
}

/// Read configuration and parse it from YAML format to Struct
fn get_config(path_to_config: String) -> Result<Config, String> {
    // It this is a systemd call
    let pathconfig = Path::new(&path_to_config);

    // Check if config exists
    if pathconfig.exists() {
        if pathconfig.is_file() {
            #[cfg(feature = "debug")]
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_BLUE,
                0,
                "CONFIG: Opened {}",
                path_to_config
            );

            // Read file
            let data;
            match fs::read_to_string(path_to_config.clone()) {
                Ok(v) => data = v,
                Err(e) => {
                    return Err(format!(
                        "Couldn't parse configuration at '{}': {}",
                        path_to_config, e
                    ))
                }
            }

            match parse_yaml(&data) {
                Ok(v) => {
                    let mut warnings = config_warnings(&v);
                    warnings.extend(unknown_keys(&data));
                    // Do basic verifications on configuration
                    match verify_config(v) {
                        Ok(vv) => {
                            for w in warnings {
                                print_debug!(
                                    PROGRAM_NAME,
                                    stderr(),
                                    COLOR_YELLOW,
                                    0,
                                    "Warning: {}",
                                    w
                                );
                            }
                            return Ok(vv);
                        }
                        Err(e) => {
                            return Err(format!("There is an error in your configuration: {}", e))
                        }
                    };
                }
                Err(e) => {
                    return Err(format!(
                        "Couldn't parse configuration at '{}': {}",
                        path_to_config, e
                    ))
                }
            }
        } else {
            let error = format!(
                "Couldn't open configuration at '{}': it is not a file!",
                path_to_config
            );
            return Err(error);
        }
    } else {
        let error = format!("Configuration at '{}' not found!", path_to_config);
        return Err(error);
    }
}

/// Parse the YAML text of a configuration, without any check of its values
fn parse_yaml(data: &str) -> Result<Config, String> {
    let mut config: Config = serde_norway::from_str(data).map_err(|e| e.to_string())?;
    // serde_yaml 0.8 (up to 1.0.8) read an empty `name:` or `channel:` as "~", so files
    // with one ran; a second, untyped read tells an empty value (None) from '' (M3R-1)
    if let Ok(raw) = serde_norway::from_str::<RawNames>(data) {
        empty_as_tilde(&mut config.name, &raw.name);
        empty_as_tilde(&mut config.channel, &raw.channel);
        for (client, names) in config
            .clients
            .iter_mut()
            .zip(raw.clients.unwrap_or_default())
        {
            empty_as_tilde(&mut client.name, &names.name);
            empty_as_tilde(&mut client.channel, &names.channel);
        }
    }
    Ok(config)
}

/// `name` and `channel` of a configuration as optional values: None for an empty value
#[derive(Deserialize)]
struct RawNames {
    name: Option<String>,
    channel: Option<String>,
    clients: Option<Vec<RawClientNames>>,
}

/// `name` and `channel` of a client. A struct of its own, without `clients`: an unknown
/// `clients` key inside a client is then ignored here as by the typed read (M3R-7)
#[derive(Deserialize)]
struct RawClientNames {
    name: Option<String>,
    channel: Option<String>,
}

/// "~" where the YAML value was empty, as serde_yaml 0.8 read it
fn empty_as_tilde(value: &mut String, raw: &Option<String>) {
    if value.is_empty() && raw.is_none() {
        *value = "~".to_string();
    }
}

/// A package held for ordering: released in `ts` order, equal `ts` in arrival order
struct Held {
    ts: u128,
    seq: u64,         // arrival order
    arrival: Instant, // released once it has waited the buffer time
    data: Vec<u8>,
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        (self.ts, self.seq) == (other.ts, other.seq)
    }
}

impl Eq for Held {}

impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Held {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.ts, self.seq).cmp(&(other.ts, other.seq))
    }
}

/// What the ordering buffer holds, behind its lock
#[derive(Default)]
struct OrderState {
    heap: BinaryHeap<Reverse<Held>>,
    seq: u64,
    finish: bool, // no more packages will come: release everything
}

/// The ordering buffer: every child puts timestamped packages in it, one sender takes them
/// out in order. The lock is held only to push or pop, never across I/O.
struct OrderBuffer {
    state: Mutex<OrderState>,
    wake: Condvar,
}

impl OrderBuffer {
    fn new() -> OrderBuffer {
        OrderBuffer {
            state: Mutex::new(OrderState::default()),
            wake: Condvar::new(),
        }
    }

    /// The state, also after a thread panicked holding it (S4-5): the heap is changed
    /// only by push and pop, which leave it whole, so the others go on with it
    fn lock(&self) -> std::sync::MutexGuard<'_, OrderState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Empties the buffer when its sender is gone: how many packages it held
    fn abandon(&self) -> u64 {
        let mut st = self.lock();
        let n = st.heap.len() as u64;
        st.heap.clear();
        n
    }

    /// Hold a package. The sender is woken only when the buffer was empty: otherwise it
    /// already waits for a head that arrived earlier, so this package cannot be due sooner.
    fn push(&self, ts: u128, data: Vec<u8>, arrival: Instant) {
        let mut st = self.lock();
        let was_empty = st.heap.is_empty();
        let seq = st.seq;
        st.seq += 1;
        st.heap.push(Reverse(Held {
            ts,
            seq,
            arrival,
            data,
        }));
        drop(st);
        if was_empty {
            self.wake.notify_one();
        }
    }

    /// Packages held now (statistics line)
    fn len(&self) -> usize {
        self.lock().heap.len()
    }

    /// finish() was called
    fn finished(&self) -> bool {
        self.lock().finish
    }

    /// No child will push again: the sender releases everything left, in order, and ends
    fn finish(&self) {
        self.lock().finish = true;
        self.wake.notify_all();
    }
}

/// Packages due at `now`, in order: the head goes once it has waited `hold` since it
/// arrived, then the next head is looked at (a recent package with a lower `ts` holds back
/// the ones behind it, as in 1.0.8). After finish() everything goes.
fn release(st: &mut OrderState, now: Instant, hold: Duration) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(Reverse(head)) = st.heap.peek() {
        // checked_add: a buffer time past the clock's range never becomes due (M3R-13); the
        // package then waits for finish()
        if st.finish || head.arrival.checked_add(hold).is_some_and(|due| due <= now) {
            out.push(st.heap.pop().unwrap().0.data);
        } else {
            break;
        }
    }
    out
}

/// When the head will be due; None when nothing is held or the head is never due
fn next_release(st: &OrderState, hold: Duration) -> Option<Instant> {
    st.heap
        .peek()
        .and_then(|Reverse(head)| head.arrival.checked_add(hold))
}

/// The sender's loop: wait for due packages and hand each batch to `turn` in order, as
/// `Some(batch)`; end once finish() was called and the buffer is empty. After a second
/// with nothing delivered it calls `turn(None)`, once a second while that lasts, so the
/// sender looks after its target links as the workers do on their idle path (L4c-3,
/// L4c-6). Otherwise it sleeps until the head is due or a push into an empty buffer
/// wakes it.
fn send_ordered(buffer: &OrderBuffer, hold: Duration, mut turn: impl FnMut(Option<Vec<Vec<u8>>>)) {
    let mut st = buffer.lock();
    let mut active = Instant::now();
    loop {
        let batch = release(&mut st, Instant::now(), hold);
        if !batch.is_empty() {
            // Deliver without the lock, so children keep pushing meanwhile
            drop(st);
            turn(Some(batch));
            active = Instant::now();
            st = buffer.lock();
            continue;
        }
        if st.finish {
            return;
        }
        let tick = active + DEFAULT_CHECK;
        if Instant::now() >= tick {
            drop(st);
            turn(None);
            active = Instant::now();
            st = buffer.lock();
            continue;
        }
        let until = match next_release(&st, hold) {
            Some(due) => cmp::min(due, tick),
            None => tick,
        };
        let wait = until.saturating_duration_since(Instant::now());
        st = buffer
            .wake
            .wait_timeout(st, wait)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// The sender thread: delivers the ordering buffer through its own target connections
fn sender(
    id: u16,
    buffer: &OrderBuffer,
    hold: Duration,
    config: Config,
    filters: Filters,
    stats: Arc<Vec<WorkerStats>>,
) {
    // One link per target, kept for the whole run: a target that cannot be reached is
    // retried at most once a second while the others keep receiving (M3R-12, L3b)
    // Nothing to deliver before the workers pop; at the stop finish() ends the wait
    let mut clients = new_targets(id, &config, &filters, || buffer.finished());
    let mut cursor: usize = 0;
    send_ordered(buffer, hold, |turn| {
        let mut counts = Counts::default();
        match turn {
            Some(batch) => {
                for package in &batch {
                    deliver(
                        id,
                        &filters.root,
                        &config,
                        &mut clients,
                        &mut cursor,
                        package,
                        &mut counts,
                    );
                }
            }
            // Nothing for a second: the idle path, as a worker's
            None => refresh(id, &mut clients, &mut counts.deleted),
        }
        report(worker_slot(&stats, id), &clients, &mut counts);
    });
}

/// One link per target, in configuration order, each with its compiled filter; every
/// target gets a first connect attempt now
fn new_targets(
    id: u16,
    config: &Config,
    filters: &Filters,
    stop: impl Fn() -> bool,
) -> Vec<RedisLink> {
    let mut clients: Vec<RedisLink> = config
        .clients
        .iter()
        .zip(&filters.clients)
        .map(|(client, filter)| new_link(id, config, client, filter))
        .collect();
    // Every first attempt at once, then wait for each up to its own bound before the
    // first pop (M3R-23 = R23a): a target that answers late at start loses nothing, as in
    // 1.0.8, and one that never answers holds the start once, by a bounded time, while the
    // packages stay in the source. A stop ends the wait at once.
    for client in clients.iter_mut() {
        client.connecting = Some(connect_in_background(id, &client.endpoint));
    }
    let start = Instant::now();
    for client in clients.iter_mut() {
        let deadline = start + start_bound(&client.endpoint);
        if let Some(rx) = client.connecting.take() {
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(cmp::min(left, START_POLL)) {
                    Ok(result) => {
                        connected(client, result);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Past its bound or stopping: the attempt goes on in the background
                        if left <= START_POLL || stop() {
                            client.connecting = Some(rx);
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        connected(client, Err("connect thread ended".to_string()));
                        break;
                    }
                }
            }
        }
    }
    clients
}

/// How long a thread waits at its start for a target's first connect (M3R-23 = R23a): its
/// connect timeout plus its command timeout (CLIENT SETNAME), 7 s by default. With either
/// at 0 (no limit) the attempt has no bound, so the wait takes the defaults' 7 s (L4c-7 =
/// a) and the attempt goes on in the background after it.
fn start_bound(endpoint: &Endpoint) -> Duration {
    match (endpoint.connect_timeout, endpoint.command_timeout) {
        (Some(connect), Some(command)) => connect + command,
        _ => DEFAULT_CONNECT_TIMEOUT + DEFAULT_COMMAND_TIMEOUT,
    }
}

/// A target's link state, not connected yet
fn new_link(id: u16, config: &Config, client: &ClientConfig, filter: &Option<Filter>) -> RedisLink {
    RedisLink {
        config: client.clone(),
        link: None,
        retry_at: None,
        backoff: TARGET_BACKOFF_MIN,
        stuck_since: None,
        packages: 0,
        last_check: None,
        filter: filter.clone(),
        request: Vec::new(),
        endpoint: Endpoint::target(client).named(connection_name(
            &config.name,
            &client.name,
            &role(id, config),
        )),
        connecting: None,
        down_since: None,
        attempts: 0,
        skip_noted: false,
    }
}

/// The role in a connection's name: `w<k>` for worker k, `sender` for the sender (its id
/// is the one after the last worker's)
fn role(id: u16, config: &Config) -> String {
    if id == config.children {
        "sender".to_string()
    } else {
        format!("w{}", id)
    }
}

/// A connect attempt in its own thread; its result comes through the returned channel.
/// If nobody waits for it any more, the link it makes is dropped with the channel.
fn connect_in_background(id: u16, endpoint: &Endpoint) -> Connecting {
    let (tx, rx) = mpsc::channel();
    let endpoint = endpoint.clone();
    thread::spawn(move || {
        let _ = tx.send(redis_connect(id, &endpoint, true));
    });
    rx
}

/// Takes the result of a connect attempt: the link, or the backoff. One line when the
/// target goes down and one when it is back; the failed retries in between print nothing
/// (S4-4 = a)
fn connected(client: &mut RedisLink, result: Result<redis::Connection, String>) -> bool {
    match result {
        Ok(link) => {
            client.link = Some(link);
            client.retry_at = None;
            client.skip_noted = false;
            if let Some(since) = client.down_since.take() {
                print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_GREEN,
                    0,
                    "Target '{}:{}@{}' is back after {:.1} s ({} failed connects)",
                    client.config.hostname,
                    client.config.port,
                    client.config.channel,
                    since.elapsed().as_secs_f64(),
                    client.attempts
                );
            }
            client.attempts = 0;
            true
        }
        Err(e) => {
            if client.down_since.is_none() {
                print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_RED,
                    0,
                    "Error while connecting to target '{}:{}@{}': {}",
                    client.config.hostname,
                    client.config.port,
                    client.config.channel,
                    e
                );
            }
            client.attempts += 1;
            mark_down(client);
            false
        }
    }
}

/// Whether the target has a link. Without one, once its retry time has come a connect
/// attempt starts in its own thread; this thread waits for it at most CONNECT_GRACE (a
/// target that answers is connected inline and takes the package) and otherwise goes on
/// without the target, picking the result up on a later package (S4-3 = a, X1c). A failed
/// attempt waits out the backoff before the next one.
fn ensure_link(id: u16, client: &mut RedisLink) -> bool {
    if client.link.is_some() {
        return true;
    }
    // An attempt already running is only looked at, never waited for again
    let (rx, grace) = match client.connecting.take() {
        Some(rx) => (rx, Duration::ZERO),
        None => {
            if client.retry_at.is_some_and(|at| Instant::now() < at) {
                return false;
            }
            (connect_in_background(id, &client.endpoint), CONNECT_GRACE)
        }
    };
    match rx.recv_timeout(grace) {
        Ok(result) => connected(client, result),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            client.connecting = Some(rx);
            false
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            connected(client, Err("connect thread ended".to_string()))
        }
    }
}

/// One line per thread and target when packages start to be skipped for a target that has
/// not connected yet and has not failed either (M3R-23): its first attempt outlasted the
/// start's wait. A target that failed has its "Error while connecting" line already.
fn note_skip(id: u16, client: &mut RedisLink) {
    if client.down_since.is_none() && !client.skip_noted {
        client.skip_noted = true;
        print_debug!(
            PROGRAM_NAME,
            stderr(),
            COLOR_YELLOW,
            0,
            "{} - {} :: {} not connected yet: its packages are skipped until it connects",
            id,
            client.config.name,
            client.config.channel
        );
    }
}

/// No link until the backoff has passed; the next failure waits twice as long, up to
/// TARGET_BACKOFF_MAX (plan 2.2). A command that succeeds resets it.
fn mark_down(client: &mut RedisLink) {
    client.down_since.get_or_insert_with(Instant::now);
    client.link = None;
    client.retry_at = Some(Instant::now() + client.backoff);
    client.backoff = cmp::min(client.backoff * 2, TARGET_BACKOFF_MAX);
}

/// A command error that leaves the connection unusable: an I/O error, or a timeout (the
/// late reply would be read as the answer to the next command)
fn link_broken(client: &mut RedisLink, e: &redis::RedisError) {
    if e.is_io_error() || e.is_timeout() || e.is_unrecoverable_error() {
        mark_down(client);
    }
}

/// After a command error on a target: a broken connection is dropped and reconnected after
/// the backoff (link_broken() did it where the error was seen); a server error (wrong type,
/// out of memory) keeps the link
fn target_failed(client: &mut RedisLink) {
    if client.link.as_ref().is_some_and(|l| !l.is_open()) {
        mark_down(client);
    }
}

/// The open link of a target (callers ensure_link() first)
fn link(link: &mut Option<redis::Connection>) -> Result<&mut redis::Connection, String> {
    link.as_mut().ok_or_else(|| "not connected".to_string())
}

/// Manage the full process from a child
fn child(
    id: u16,
    ordering_regex: Option<Regex>,
    ordering_limit: Option<usize>,
    stats: Arc<Vec<WorkerStats>>,
    stop: Arc<AtomicBool>,
    ordered: Option<Arc<OrderBuffer>>,
    config: Config,
    filters: Filters,
) {
    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_BLUE,
        0,
        "Child {}: Starts",
        id
    );

    // Prepare main variable
    let mut keepworking = true;
    let mut request_finish = false;

    // The targets live as long as the worker: a source error reconnects the source only,
    // and a target that cannot be reached does not keep the worker from the others (L3b)
    let mut clients = new_targets(id, &config, &filters, || stop.load(Ordering::Relaxed));
    // `-`: a source connection has no target
    let source_endpoint =
        Endpoint::source(&config).named(connection_name(&config.name, "-", &format!("src{}", id)));

    // Prepare the retention data
    while keepworking {
        // Stopped while (re)connecting: a worker holds nothing between packages
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Connect to source
        let mut source: redis::Connection;
        let mut error = false;
        match redis_connect(id, &source_endpoint, false) {
            Ok(link) => {
                source = link;

                {
                    // Keep working while allowed
                    let mut counts = Counts::default();
                    let mut cursor: usize = 0; // spreader: next target to try
                                               // The BLPOP request never changes: encoded once per connection (M3-o)
                    let blpop = blpop_cmd(&config.channel).get_packed_command();
                    let mut lasttime = get_current_time();
                    while keepworking {
                        // Check if we should save statistics
                        if (get_current_time() - 1) > lasttime {
                            report(worker_slot(&stats, id), &clients, &mut counts);
                            lasttime = get_current_time();
                        }

                        // Check if we got requested to finish
                        if !request_finish {
                            // Get a new package
                            let item: redis::RedisResult<redis::Value> = source
                                .req_packed_command(&blpop)
                                .and_then(|v| v.extract_error());
                            match item {
                                Ok(redis::Value::Nil) => {
                                    // Nothing came: refresh the targets' limits state
                                    refresh(id, &mut clients, &mut counts.deleted);
                                }
                                Ok(redis::Value::Array(data)) => {
                                    // This is the expected data
                                    // println!("Bulk(Vec<Value>)");

                                    // Send to all clients
                                    counts.incoming += 1;

                                    // The package is the second element (key, value); taken by
                                    // value, so the payload bytes are moved, not copied
                                    if let Some(redis::Value::BulkString(raw_bdata)) =
                                        data.into_iter().nth(1)
                                    {
                                        // With ordering a timestamped package goes to the
                                        // buffer (moved, not copied) and the sender delivers it;
                                        // any other package the worker delivers itself
                                        let held = match (&ordered, &ordering_regex) {
                                            (Some(buffer), Some(re)) => {
                                                match ordering_ts(re, ordering_limit, &raw_bdata) {
                                                    Ok(ts) => {
                                                        buffer.push(ts, raw_bdata, Instant::now());
                                                        None
                                                    }
                                                    Err(warning) => {
                                                        print_debug!(
                                                            PROGRAM_NAME,
                                                            stderr(),
                                                            COLOR_YELLOW,
                                                            0,
                                                            "{}",
                                                            warning
                                                        );
                                                        Some(raw_bdata)
                                                    }
                                                }
                                            }
                                            _ => Some(raw_bdata),
                                        };
                                        if let Some(package) = held {
                                            deliver(
                                                id,
                                                &filters.root,
                                                &config,
                                                &mut clients,
                                                &mut cursor,
                                                &package,
                                                &mut counts,
                                            );
                                        }
                                    }
                                }
                                Ok(_) => {
                                    // Any other reply means the key is not a list (BLPOP answers
                                    // nil or an array; other types only on a wrong server)
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': not a queue!", config.hostname, config.port);
                                    error = true;
                                }
                                Err(e) => {
                                    // There was an error
                                    print_debug!(
                                        PROGRAM_NAME,
                                        stderr(),
                                        COLOR_RED,
                                        0,
                                        "Error while reading from Redis Server '{}:{}': {}",
                                        config.hostname,
                                        config.port,
                                        e
                                    );
                                    error = true;
                                }
                            }

                            // Check if our father wants us to finish
                            if stop.load(Ordering::Relaxed) {
                                request_finish = true;
                            }
                        } else {
                            // A worker holds nothing between packages (the ordering buffer is
                            // the sender's): done
                            keepworking = false;
                        }

                        // Error found while processing
                        if error {
                            pause(|| stop.load(Ordering::Relaxed), Duration::from_secs(1));
                            break;
                        }
                    }

                    // Report what is left of this connection's counts
                    report(worker_slot(&stats, id), &clients, &mut counts);
                }
            }
            Err(e) => {
                // Show error
                print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_RED,
                    0,
                    "Error while connecting to source Redis Server '{}:{}': {}",
                    config.hostname,
                    config.port,
                    e
                );
                pause(|| stop.load(Ordering::Relaxed), Duration::from_secs(1));
            }
        }

        // Check if our father wants us to finish
        if stop.load(Ordering::Relaxed) {
            keepworking = false;
        }
    }

    #[cfg(feature = "debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Ends", id);
}

/// Whether SIGHUP is ignored now (inherited from the parent, as under nohup)
fn sighup_ignored() -> bool {
    // SAFETY: with a null new action, sigaction only reads the current disposition
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut current) == 0
            && current.sa_sigaction == libc::SIG_IGN
    }
}

/// Ignore SIGHUP again after ctrlc installed its handler for it
fn ignore_sighup() {
    // SAFETY: SIG_IGN installs no handler code
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
}

/// Wait `time`, or less once `stopping` says the program is stopping
fn pause(stopping: impl Fn() -> bool, time: Duration) {
    let until = Instant::now() + time;
    while !stopping() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// The statistics line and the status file: every worker's and the sender's counts since
/// the last call, `lasttime` (ms), as rates and as interval totals
fn statistics(
    name: &str,
    ordered: Option<&OrderBuffer>,
    clients: &[ClientConfig],
    stats: &[WorkerStats],
    statusfile: &Option<String>,
    lasttime: u128,
    since_start: &mut Counts,
) {
    // Counts of every child since the last report
    let counts = take_counts(stats);
    since_start.add(&counts);
    let Counts {
        incoming,
        outgoing,
        dropped,
        deleted,
        filtered,
        stuck,
        failed,
    } = counts;

    // Show statistics
    // The final call can come right after a periodic one: keep the rates finite
    let diff: f64 = (((get_current_time_with_ms() - lasttime) as f64) / 1000.0).max(0.001);
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_BLUE,
        COLOR_NOTAIL,
        "{} - ",
        name
    );
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        COLOR_NOHEAD_NOTAIL,
        "Incoming: {:.1} regs/sec",
        (incoming as f64) / diff
    );
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_WHITE,
        COLOR_NOHEAD_NOTAIL,
        " | "
    );
    if ordered.is_some() {
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_GREEN,
            COLOR_NOHEAD_NOTAIL,
            "Queue: {} regs",
            ordered.map_or(0, |b| b.len())
        );
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_WHITE,
            COLOR_NOHEAD_NOTAIL,
            " | "
        );
    }
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_GREEN,
        COLOR_NOHEAD_NOTAIL,
        "Outgoing: {:.1} regs/sec",
        (outgoing as f64) / diff
    );
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_WHITE,
        COLOR_NOHEAD_NOTAIL,
        " | "
    );
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_RED,
        COLOR_NOHEAD_NOTAIL,
        "Dropped: {:.1} regs/sec",
        (dropped as f64) / diff
    );
    if deleted > 0 {
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_WHITE,
            COLOR_NOHEAD_NOTAIL,
            " | "
        );
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_YELLOW,
            COLOR_NOHEAD_NOTAIL,
            "Deleted: {:.1} regs/sec",
            (deleted as f64) / diff
        );
    }

    // Show stuck clients
    let stucks: Vec<String> = stuck_clients(stats, clients.len())
        .into_iter()
        .map(|c| {
            let client = &clients[c];
            format!("{}:{}", client.name, client.channel)
        })
        .collect();
    if stucks.len() > 0 {
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_YELLOW,
            COLOR_NOHEAD_NOTAIL,
            "  -> Stucked: [ {} ]",
            stucks.join(", ")
        );
    }

    // Write statistics: every 1.0.8 key with its meaning, then the breakdown of `drop`,
    // the targets stuck and down, the counts since the start (M3-e = ea)
    if let Some(status) = statusfile {
        let names = |indexes: Vec<usize>| -> Vec<String> {
            indexes
                .into_iter()
                .map(|c| format!("{}:{}", clients[c].name, clients[c].channel))
                .collect()
        };
        let rate = |n: u64| (n as f64) / diff;
        let stat = json!({
            "date": get_current_time(),
            "in": rate(incoming),
            "out": rate(outgoing),
            "drop": rate(dropped),
            "deleted": rate(deleted),
            "total_in": incoming,
            "total_out": outgoing,
            "total_drop": dropped,
            "total_deleted": deleted,
            "filtered": rate(filtered),
            "stuck": rate(stuck),
            "failed": rate(failed),
            "total_filtered": filtered,
            "total_stuck": stuck,
            "total_failed": failed,
            "queue": ordered.map_or(0, |b| b.len()),
            "stuck_targets": stucks,
            "down_targets": names(down_clients(stats, clients.len())),
            "since_start": {
                "in": since_start.incoming,
                "out": since_start.outgoing,
                "drop": since_start.dropped,
                "deleted": since_start.deleted,
                "filtered": since_start.filtered,
                "stuck": since_start.stuck,
                "failed": since_start.failed,
            },
            "version": VERSION,
        });
        if let Err(e) = write_status(Path::new(status), stat.to_string().as_bytes()) {
            print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_RED,
                0,
                "Unable to write STATUS at {}: {}",
                status,
                e
            );
        }
    }

    // Show tail
    print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD, "");
}

/// Set once the status file had to be written in place (one warning, D4d-4 = a)
static STATUS_IN_PLACE: AtomicBool = AtomicBool::new(false);

/// Writes the status file whole (M3-e = ea): the text goes to `<file>.tmp` beside the file
/// and is renamed over it, so a reader never sees half of it (an in-place write truncates,
/// then writes). What works today keeps working (D4d-4 = a, COORD 2026-11-14 20:53):
/// - a status path that is a symbolic link stays a link: the file it points to is replaced;
/// - an existing file keeps its mode, owner and group (set on the temporary file first);
/// - where the temporary file cannot be made, given that owner and group, or renamed (a
///   directory the user may not write, a file owned by another user), the file is written
///   in place, as before, with one warning the first time.
fn write_status(path: &Path, text: &[u8]) -> io::Result<()> {
    match write_replacing(path, text) {
        Ok(()) => Ok(()),
        Err(why) => {
            if !STATUS_IN_PLACE.swap(true, Ordering::Relaxed) {
                print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_YELLOW,
                    0,
                    "Warning: the status file {} is written in place, not replaced whole: {}",
                    path.display(),
                    why
                );
            }
            fs::write(path, text)
        }
    }
}

/// The atomic write of write_status; any error leaves the file as it was
fn write_replacing(path: &Path, text: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // A link is followed to the file it points to (a dangling one is an error here)
    let target = if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let mut tmp = target.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let old = fs::metadata(&target).ok();
    let result = (|| {
        fs::write(&tmp, text)?;
        if let Some(old) = &old {
            let new = fs::metadata(&tmp)?;
            if (new.uid(), new.gid()) != (old.uid(), old.gid()) {
                std::os::unix::fs::chown(&tmp, Some(old.uid()), Some(old.gid()))?;
            }
            fs::set_permissions(&tmp, fs::Permissions::from_mode(old.mode() & 0o7777))?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// A worker's counts since its last report (then reset) and the state of its targets:
/// stuck, and without a link (down, or not connected yet)
fn report(slot: &WorkerStats, clients: &[RedisLink], counts: &mut Counts) {
    slot.add(counts);
    *counts = Counts::default();
    for ((stuck, down), client) in slot
        .stuck_targets
        .iter()
        .zip(&slot.down_targets)
        .zip(clients)
    {
        stuck.store(client.stuck_since.is_some(), Ordering::Relaxed);
        down.store(client.link.is_none(), Ordering::Relaxed);
    }
}

fn redis_connect(
    _id: u16,
    endpoint: &Endpoint,
    is_client: bool,
) -> Result<redis::Connection, String> {
    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        if is_client { COLOR_CYAN } else { COLOR_BLUE },
        0,
        "Child {}: Connecting to {} {}:{} (tls={}, db={}, user={:?})",
        _id,
        if is_client { "client" } else { "source" },
        endpoint.host,
        endpoint.port,
        endpoint.ssl,
        endpoint.database,
        endpoint.username
    );

    let client = redis::Client::open(endpoint.info())
        .map_err(|e| format!("Invalid connection settings: {}", e))?;
    // A server that drops packets instead of refusing would hold the thread for the
    // kernel's connect timeout (about 2 min), and one that stops answering would hold it
    // for ever: bounded unless the configuration asks for the old wait (M3R-12 X1b, M3R-19)
    let link = match endpoint.connect_timeout {
        Some(t) => client.get_connection_with_timeout(t),
        None => client.get_connection(),
    }
    .map_err(|e| format!("Couldn't connect to Redis Server: {}", e))?;
    // The source's BLPOP waits up to its own timeout before the server answers
    let read = endpoint
        .command_timeout
        .map(|t| if is_client { t } else { t + BLPOP_WAIT });
    link.set_read_timeout(read)
        .and_then(|_| link.set_write_timeout(endpoint.command_timeout))
        .map_err(|e| format!("Couldn't set the timeouts: {}", e))?;
    let mut link = link;
    if let Some(name) = &endpoint.name {
        // The name is for operators: refused (ACL, renamed command) it is skipped; a peer
        // that closes on the command (a proxy, M3R-2) gets a fresh connection without it;
        // one that does not answer within the command timeout is not usable at all
        if let Err(e) = redis::cmd("CLIENT")
            .arg("SETNAME")
            .arg(name)
            .query::<()>(&mut link)
        {
            if e.is_timeout() {
                return Err(format!("no answer to CLIENT SETNAME: {}", e));
            }
            if e.is_io_error() || !link.is_open() {
                let unnamed = Endpoint {
                    name: None,
                    ..endpoint.clone()
                };
                return redis_connect(_id, &unnamed, is_client);
            }
        }
    }
    Ok(link)
}

/// BLPOP with a one second timeout, sent as an integer as redis 0.19 did:
/// Commands::blpop takes an f64 and sends "1.0", which servers before 6.0 reject
fn blpop_cmd(channel: &str) -> redis::Cmd {
    let mut cmd = redis::cmd("BLPOP");
    cmd.arg(channel).arg(BLPOP_WAIT.as_secs());
    cmd
}

fn send_to_client(client: &mut RedisLink, data: &[u8]) -> Result<(), String> {
    // Borrowed: config and link are distinct fields
    let channel = &client.config.channel;

    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_WHITE,
        0,
        "RPUSH {} bytes to '{}'!",
        data.len(),
        channel
    );

    // Encoded into the target's reused buffer and sent as is (M3-o): no command object, no
    // allocation and one copy of the payload; the crate still reads and checks the reply
    pack_rpush(&mut client.request, channel.as_bytes(), data);
    match link(&mut client.link)?
        .req_packed_command(&client.request)
        .and_then(|v| v.extract_error())
    {
        Ok(_) => {
            client.backoff = TARGET_BACKOFF_MIN;
            Ok(())
        }
        Err(e) => {
            link_broken(client, &e);
            Err(format!("couldn't push to channel: {}", e))
        }
    }
}

/// `RPUSH key value` in RESP, written into `buf` (cleared first): the bytes the crate's
/// command builder produces, without building a command
fn pack_rpush(buf: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    buf.clear();
    buf.extend_from_slice(b"*3\r\n$5\r\nRPUSH\r\n");
    for arg in [key, value] {
        // Writing into a Vec cannot fail
        let _ = write!(buf, "${}\r\n", arg.len());
        buf.extend_from_slice(arg);
        buf.extend_from_slice(b"\r\n");
    }
}

/// Whether the queue of a target is due a check (L1a: monotonic clock, `>=`): never
/// checked yet, `timelimit` seconds since the last check, or the countdown at 0. A target
/// without `timelimit` and `checklimit` is checked every `DEFAULT_CHECK`.
fn can_check_queue(
    timelimit: Option<u64>,
    checklimit: Option<u64>,
    packages: u64,
    last_check: Option<Instant>,
    now: Instant,
) -> bool {
    let since = match last_check {
        None => return true,
        Some(at) => now.saturating_duration_since(at),
    };
    if (timelimit == None) && (checklimit == None) {
        return since >= DEFAULT_CHECK;
    }
    if let Some(t) = timelimit {
        if since >= Duration::from_secs(t) {
            return true;
        }
    }
    checklimit.is_some() && packages == 0
}

fn can_send(id: u16, client: &mut RedisLink, deleted: &mut u64) -> Result<bool, String> {
    // Without a hardlimit a target has no limits: nothing would read its length, so no
    // LLEN (1.0.8 sent one every second per target and thread, M3F-7)
    if client.config.hardlimit.is_none() {
        return Ok(true);
    }

    // Check if we can check queue
    if can_check_queue(
        client.config.timelimit,
        client.config.checklimit,
        client.packages,
        client.last_check,
        Instant::now(),
    ) {
        // Reset timers
        client.last_check = Some(Instant::now());
        match client.config.checklimit {
            None => client.packages = 0,
            Some(v) => client.packages = v,
        }

        // Let's check the queue (lengths as 64 bits: i32 misread lists above 2^31, M3F-5)
        let result: redis::RedisResult<u64> = link(&mut client.link)?.llen(&client.config.channel);
        match result {
            Ok(len) => {
                if client.config.hardlimit != None {
                    if client.stuck_since.is_none() {
                        // The client is not sleeping
                        if len >= client.config.hardlimit.unwrap() {
                            if client.config.deleteblock == None {
                                // We lock the client
                                client.stuck_since = Some(Instant::now());
                                print_debug!(
                                    PROGRAM_NAME,
                                    stderr(),
                                    COLOR_RED,
                                    0,
                                    "{} - {} :: {} stuck! (Len: {})",
                                    id,
                                    client.config.name,
                                    client.config.channel,
                                    len
                                );
                            } else {
                                // Deleteblock in action: drop the oldest `deleteblock`
                                // elements, to the end of the list (1.0.8 kept at most
                                // 100000000 after them, M3F-5)
                                let block = client.config.deleteblock.unwrap();
                                let start = isize::try_from(block).unwrap_or(isize::MAX);
                                let mut actual_len: u64 = len;
                                while actual_len >= client.config.hardlimit.unwrap() {
                                    // Trim elements from the queue
                                    let result: redis::RedisResult<redis::Value> = link(
                                        &mut client.link,
                                    )?
                                    .ltrim(&client.config.channel, start, -1);
                                    match result {
                                        // What it removed, not the block size when the list
                                        // held fewer (L4c-4)
                                        Ok(_) => *deleted += cmp::min(block, actual_len),
                                        Err(e) => {
                                            link_broken(client, &e);
                                            return Err(format!("error in deleteblock while deleting block with {} elements: {}", client.config.deleteblock.unwrap(), e));
                                        }
                                    }

                                    // Read len again
                                    let result: redis::RedisResult<u64> =
                                        link(&mut client.link)?.llen(&client.config.channel);
                                    match result {
                                        Ok(len) => actual_len = len,
                                        Err(e) => {
                                            link_broken(client, &e);
                                            return Err(format!("error in deleteblock while requesting the length to the channel: {}", e));
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // The client is sleeping (stuck)
                        if len < client.config.softlimit.unwrap() {
                            // We lock the client
                            client.stuck_since = None;
                            print_debug!(
                                PROGRAM_NAME,
                                stderr(),
                                COLOR_GREEN,
                                0,
                                "{} - {} :: {} freed! (Len: {})",
                                id,
                                client.config.name,
                                client.config.channel,
                                len
                            );
                        }
                    }
                }

                #[cfg(feature = "debug")]
                {
                    if client.stuck_since.is_none() {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   true", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    } else {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   false", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    }
                }

                // If not stuck, can keep sending
                return Ok(client.stuck_since.is_none());
            }
            Err(e) => {
                link_broken(client, &e);
                return Err(format!("error requesting the length to the channel: {}", e));
            }
        };
    } else {
        // Count down packages, only with a checklimit (without one the counter was 0
        // after every check and this underflowed: a panic in a debug build, A-C4, M1F-1)
        if client.config.checklimit.is_some() {
            client.packages = client.packages.saturating_sub(1);
        }

        #[cfg(feature = "debug")]
        {
            if client.stuck_since.is_none() {
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_GREEN,
                    0,
                    "{}: can_send(): not stucked :: packages={}   =>   true",
                    id,
                    client.packages
                );
            } else {
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_RED,
                    0,
                    "{}: can_send(): stucked :: packages={}   =>   false",
                    id,
                    client.packages
                );
            }
        }

        // Return whatever is the status of the queue (we can not check it out)
        return Ok(client.stuck_since.is_none());
    }
}

fn send(
    id: u16,
    client: &mut RedisLink,
    dirty_bdata: &[u8],
    deleted: &mut u64,
) -> Result<Sent, String> {
    let bdata = match &client.filter {
        Some(f) => match f.apply(dirty_bdata) {
            Some(b) => b,
            None => return Ok(Sent::Filtered),
        },
        None => Cow::Borrowed(dirty_bdata),
    };
    // If we can send to this queue
    match can_send(id, client, deleted) {
        // Allowed to send
        Ok(true) => match send_to_client(client, &bdata) {
            Ok(_) => Ok(Sent::Taken),
            Err(e) => Err(format!("error while sending to the client: {}", e)),
        },

        // Not allowed to send
        Ok(false) => Ok(Sent::Stuck),

        // There was an error
        Err(e) => Err(format!("error while checking queue: {}", e)),
    }
}

/// Timestamp of a package for ordering: the named group `ts` of the ordering regex, looked
/// for in the first `limit` bytes (0 or none: the whole package) and parsed as an integer.
/// The error is the warning to print; the package is then sent without ordering.
fn ordering_ts(re: &Regex, limit: Option<usize>, data: &[u8]) -> Result<u128, &'static str> {
    let haystack = match limit {
        Some(l) if l > 0 => &data[..cmp::min(l, data.len())],
        _ => data,
    };
    match re.captures(haystack) {
        Some(x) => {
            let parsed_ts = x.name("ts").map_or(&b""[..], |m| m.as_bytes());
            std::str::from_utf8(parsed_ts)
                .ok()
                .and_then(|s| s.parse::<u128>().ok())
                .ok_or(
                    "Found a package with ordering information but 'ts' couldn't be parsed to u128",
                )
        }
        None => Err("Found a package without ordering information"),
    }
}

/// Deliver one package to the targets the mode selects, through the root and client filters
/// and the limits; counted as outgoing when at least one target took it, else as dropped.
/// A target without a link is skipped (not taken) until its retry time; one whose command
/// fails keeps or loses its link as target_failed() decides.
fn deliver(
    id: u16,
    root_filter: &Option<Filter>,
    config: &Config,
    clients: &mut [RedisLink],
    cursor: &mut usize,
    package: &[u8],
    counts: &mut Counts,
) {
    let bdata = match root_filter {
        Some(f) => f.apply(package),
        None => Some(Cow::Borrowed(package)),
    };
    // Why the targets tried did not take it: one that took it ends the matter
    let mut why = Why::default();
    let deleted = &mut counts.deleted;
    let mut attempt = |client: &mut RedisLink, bdata: &[u8]| {
        if !ensure_link(id, client) {
            note_skip(id, client);
            // Failed only if its filter would have taken it (D4d-3 = a)
            if client
                .filter
                .as_ref()
                .is_none_or(|f| f.apply(bdata).is_some())
            {
                why.failed = true;
            }
            return false;
        }
        match send(id, client, bdata, deleted) {
            Ok(Sent::Taken) => true,
            Ok(Sent::Filtered) => false,
            Ok(Sent::Stuck) => {
                why.stuck = true;
                false
            }
            Err(e) => {
                print_debug!(
                    PROGRAM_NAME,
                    stderr(),
                    COLOR_RED,
                    0,
                    "Error while sending to '{}:{}@{}': {}",
                    config.hostname,
                    config.port,
                    config.channel,
                    e
                );
                target_failed(client);
                why.failed = true;
                false
            }
        }
    };
    let delivered = match bdata {
        None => false,
        Some(bdata) => {
            if config.mode == "replicant" {
                // Every target, in configuration order
                let mut sent = false;
                for client in clients.iter_mut() {
                    sent |= attempt(client, &bdata);
                }
                sent
            } else {
                spread(cursor, clients.len(), |i| attempt(&mut clients[i], &bdata)).is_some()
            }
        }
    };
    if delivered {
        counts.outgoing += 1;
    } else {
        // One reason per package (D4d-3 = a): filtered when no target accepted it, stuck
        // when every target that accepted it was stuck, failed otherwise
        counts.dropped += 1;
        if why.failed {
            counts.failed += 1;
        } else if why.stuck {
            counts.stuck += 1;
        } else {
            counts.filtered += 1;
        }
    }
}

/// What the targets that did not take a package said: some was stuck, some failed (down,
/// not connected yet, an error); neither means every filter missed
#[derive(Default)]
struct Why {
    stuck: bool,
    failed: bool,
}

/// What one target did with a package
#[derive(Debug, PartialEq)]
enum Sent {
    Taken,
    /// Its filter did not accept the package
    Filtered,
    /// Stuck at its limits
    Stuck,
}

/// Spreader target selection: try the targets in turn from `cursor`, at most one full turn;
/// the first that takes the package wins and the next package starts after it. When none
/// takes it the cursor stays. Same turn as 1.0.8, which rotated the list by one per attempt.
fn spread(cursor: &mut usize, n: usize, mut attempt: impl FnMut(usize) -> bool) -> Option<usize> {
    for k in 0..n {
        let i = (*cursor + k) % n;
        if attempt(i) {
            *cursor = (i + 1) % n;
            return Some(i);
        }
    }
    None
}

/// The idle path, at each empty BLPOP (once a second): a target that is down gets its
/// reconnect attempt; a stuck target is re-checked, at most once a second, whatever its
/// timelimit and checklimit say, so it is freed while nothing comes (L4c-3); every other
/// connected target gets a PING, which finds a link that broke while the source was idle
/// (a target restarted) before the next packages meet it (L4c-6). 1.0.8 ran the full
/// check of every target here: an LLEN whose answer nothing read for a target without
/// limits, and one taken from the countdown as if a package had gone (M3F-7). A failing
/// target is handled alone, the worker goes on.
fn refresh(id: u16, clients: &mut [RedisLink], deleted: &mut u64) {
    for client in clients.iter_mut() {
        if !ensure_link(id, client) {
            continue;
        }
        let result = if client.stuck_since.is_some() {
            // Re-checked less than a second ago: not yet
            if client
                .last_check
                .is_some_and(|at| at.elapsed() < DEFAULT_CHECK)
            {
                continue;
            }
            // Never checked counts as due: this check runs whatever the limits say
            client.last_check = None;
            // Only the refresh matters, not whether it could send
            can_send(id, client, deleted).map(|_| ())
        } else {
            ping(client)
        };
        if let Err(e) = result {
            print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_RED,
                0,
                "{}: couldn't check queue for {}: {}",
                id,
                client.config.channel,
                e
            );
            target_failed(client);
        }
    }
}

/// A PING on a target's link; an error that broke the link drops it. An error reply
/// (a user without the right to PING, a server LOADING) came over a working link: alive,
/// nothing to say (M3R-29 = R29a; 4c printed a line for it every idle second)
fn ping(client: &mut RedisLink) -> Result<(), String> {
    let result: redis::RedisResult<String> = redis::cmd("PING").query(link(&mut client.link)?);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            link_broken(client, &e);
            if client.link.is_some() {
                Ok(())
            } else {
                Err(format!("error on PING: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests;
