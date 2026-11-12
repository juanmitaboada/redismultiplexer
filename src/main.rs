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
use std::io::{stderr, stdout, Write};
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
use std::path::Path;

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
    username: Option<String>, // Redis 6 ACL user (M3-i)
    database: Option<u32>,    // SELECT on connect, 0 when absent (M1F-10)
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
    username: Option<String>, // Redis 6 ACL user (M3-i)
    database: Option<u32>,    // SELECT on connect, 0 when absent (M1F-10)
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
    sleeping_from: u64,              // If queue is stuck, when did it happened
    packages: u64,                   // Packages we have seen from last check
    lastcheck: u64,                  // When was the last check of queue's size (time limit)
    filter: Option<Filter>,          // Client filter, compiled at start
    request: Vec<u8>,                // RPUSH request buffer, reused for every package
}

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
}

impl Endpoint {
    fn new(
        ssl: Option<bool>,
        host: &str,
        port: u16,
        username: &Option<String>,
        password: &str,
        database: Option<u32>,
    ) -> Endpoint {
        Endpoint {
            ssl: ssl.unwrap_or(false),
            host: host.to_string(),
            port,
            // Empty means none, as the URL path read an empty user or password: no AUTH
            username: username.clone().filter(|u| !u.is_empty()),
            password: Some(password.to_string()).filter(|p| !p.is_empty()),
            database: database.unwrap_or(0),
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

/// Counters of one worker, written by that worker only and read by main. Relaxed atomics:
/// every counter stands alone and main only needs a recent value. Aligned to a cache line,
/// so two workers never write the same line.
#[repr(align(64))]
struct WorkerStats {
    incoming: AtomicU64,
    outgoing: AtomicU64,
    dropped: AtomicU64,
    deleted: AtomicU64,
    stuck: Vec<AtomicBool>, // per client, in configuration order
}

impl WorkerStats {
    fn new(clients: usize) -> WorkerStats {
        WorkerStats {
            incoming: AtomicU64::new(0),
            outgoing: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            deleted: AtomicU64::new(0),
            stuck: (0..clients).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Add the counts a worker gathered since its last report
    fn add(&self, incoming: u64, outgoing: u64, dropped: u64, deleted: u64) {
        self.incoming.fetch_add(incoming, Ordering::Relaxed);
        self.outgoing.fetch_add(outgoing, Ordering::Relaxed);
        self.dropped.fetch_add(dropped, Ordering::Relaxed);
        self.deleted.fetch_add(deleted, Ordering::Relaxed);
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

/// Counts of every worker since the last call (each slot is reset): incoming, outgoing,
/// dropped, deleted
fn take_counts(stats: &[WorkerStats]) -> (u64, u64, u64, u64) {
    stats.iter().fold((0, 0, 0, 0), |t, w| {
        (
            t.0 + w.incoming.swap(0, Ordering::Relaxed),
            t.1 + w.outgoing.swap(0, Ordering::Relaxed),
            t.2 + w.dropped.swap(0, Ordering::Relaxed),
            t.3 + w.deleted.swap(0, Ordering::Relaxed),
        )
    })
}

/// Indexes of the clients that some worker found stuck at its last report
fn stuck_clients(stats: &[WorkerStats], clients: usize) -> Vec<usize> {
    (0..clients)
        .filter(|&c| stats.iter().any(|w| w.stuck[c].load(Ordering::Relaxed)))
        .collect()
}

/// Main module will manage the basics from this program
fn main() {
    // Get args
    let args: Vec<String> = std::env::args().collect();

    // Check if we got instructed with the path to configuration
    if args.len() > 1 {
        // Check if version is requested
        if (args[1] == "version") || (args[1] == "--version") {
            // Answer with our version
            println!("{} v{}", PROGRAM_NAME, VERSION);
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

                    // Render main ordering regex
                    let ordering_regex: Option<Regex>;
                    let regex_str: &str;
                    if let Some(r) = inconfig.ordering.clone() {
                        regex_str = &r;
                        let filter_temp = Regex::new(&regex_str);
                        match filter_temp {
                            Ok(f) => {
                                ordering_regex = Some(f);
                            }
                            Err(e) => {
                                print_debug!(
                                    PROGRAM_NAME,
                                    stderr(),
                                    COLOR_RED,
                                    0,
                                    "Ordering Regex '{}' doesn't compile: {}",
                                    r,
                                    e
                                );
                                ordering_regex = None;
                                error = true;
                            }
                        }
                    } else {
                        ordering_regex = None;
                    }

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

                        // Keep working while all children keep working
                        let mut lasttime = get_current_time_with_ms();
                        let mut keepworking = true;
                        while keepworking {
                            // Wait up to a second for a signal (instead of sleeping, so a
                            // stop is seen at once)
                            if keepworking_rx.recv_timeout(Duration::from_secs(1)).is_ok() {
                                keepworking = false;
                            }

                            // If nothind changed with children
                            if keepworking {
                                // A child only ends when told to: one that ended on its own
                                // (a panic) stops the program
                                if handles.iter().any(|h| h.is_finished()) {
                                    keepworking = false;
                                }

                                // Check if we can keep working
                                if keepworking {
                                    // Check if the sender has finished
                                    if sender_handle.as_ref().is_some_and(|h| h.is_finished()) {
                                        // Some child already died, show message and leave
                                        print_debug!(
                                            PROGRAM_NAME,
                                            stdout(),
                                            COLOR_YELLOW,
                                            0,
                                            "The ordered sender closed abruptly, closing everything!"
                                        );
                                        break;
                                    }

                                    // Check if we should show statistics
                                    if (get_current_time_with_ms() - (STATISTICS_SECONDS * 1000))
                                        > lasttime
                                    {
                                        statistics(
                                            &inconfig.name,
                                            ordered.as_deref(),
                                            &inconfig.clients,
                                            &stats,
                                            &statusfile,
                                            lasttime,
                                        );
                                        lasttime = get_current_time_with_ms();
                                    }
                                } else {
                                    print_debug!(
                                        PROGRAM_NAME,
                                        stdout(),
                                        COLOR_YELLOW,
                                        0,
                                        "Some child closed abruptly, closing everything!"
                                    );
                                }
                            }
                        }

                        // Tell all children to finish their job
                        stop.store(true, Ordering::Relaxed);

                        // Wait for the children: each leaves after its current package (a
                        // BLPOP waits at most 1 s); say how many are left every second
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Closing...");
                        let mut said = Instant::now();
                        while !handles.iter().all(|h| h.is_finished()) {
                            if said.elapsed() >= Duration::from_secs(1) {
                                print_debug!(
                                    PROGRAM_NAME,
                                    stdout(),
                                    COLOR_YELLOW,
                                    0,
                                    "There are {} threads left!",
                                    handles.iter().filter(|h| !h.is_finished()).count()
                                );
                                said = Instant::now();
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                        for handle in handles {
                            // A child that panicked was reported by the loop above
                            let _ = handle.join();
                        }

                        // Every child has left, so nothing more enters the ordering buffer:
                        // the sender flushes it in ts order and ends
                        if let (Some(buffer), Some(handle)) = (&ordered, sender_handle) {
                            buffer.finish();
                            handle.join().unwrap();
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
                        );

                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "Program finished!");
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

    // === FILTERS ===

    // Filter
    match &source.filter {
        None => {
            if (source.filter_until != None)
                || (source.filter_until_included != None)
                || (source.filter_limit != None)
                || (source.filter_replace != None)
                || (source.filter_replace_header != None)
            {
                return Err(format!(
                    "Source '{}' is using some filtering option but filter is not defined",
                    source.name
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

    // If some config is set, all must be set
    let mut configured = 0;

    // Ordering Regex
    match &source.ordering {
        None => (),
        Some(v) => configured += (v.len() > 0) as i8,
    }

    // Ordering Buffer Time
    match source.ordering_buffer_time {
        None => (),
        Some(v) => configured += (v > 0) as i8,
    }

    // Ordering Limit
    match source.ordering_limit {
        None => (),
        Some(v) => configured += (v > 0) as i8,
    }

    // Show error if any
    if (configured > 0) && (configured < 2) {
        return Err(format!("Source '{}' is using ordering, so you must set all ordering configuration: ordering (not empty), ordering_buffer_time (bigger than 0) and ordering_limit (bigger than 0)", source.name));
    }

    // Verify there are clients
    if config.clients.len() > 0 {
        // Verify all clients
        for client in &config.clients {
            // Verify that name is not empty
            if client.hostname.len() == 0 {
                return Err(format!(
                    "Client '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]",
                    client.name, client.hostname, client.port, client.channel
                ));
            }

            // Verify that hostname is not empty
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

            // Verify that source and target are not the same
            if (source.hostname == client.hostname)
                && (source.port == client.port)
                && (source.channel == client.channel)
            {
                return Err(format!("Client '{}' is using same connection information than source [hostname=\"{}\", port={}, channel=\"{}\"]",client.name, client.hostname, client.port, client.channel));
            }

            // === LIMITS ===

            // If some config is set, all must be set
            let mut configured = 0;

            // Timelimit
            match client.timelimit {
                None => (),
                Some(0) => (),
                _ => configured += 1,
            }

            // Checklimit
            match client.checklimit {
                None => (),
                Some(0) => (),
                _ => configured += 1,
            }

            // Softlimit
            match client.softlimit {
                None => (),
                Some(0) => (),
                _ => configured += 1,
            }

            // Hardlimit
            match client.hardlimit {
                None => (),
                Some(0) => (),
                _ => configured += 1,
            }

            // Show error if any
            if (configured > 0) && (configured < 4) {
                return Err(format!("Client '{}' is using limits, so you must set all limits: timelimit, checklimit, softlimit and hardlimit to be bigger than 0", client.name));
            }

            // === FILTERS ===

            // Filter
            match &client.filter {
                None => {
                    if (client.filter_until != None)
                        || (client.filter_until_included != None)
                        || (client.filter_limit != None)
                        || (client.filter_replace != None)
                        || (client.filter_replace_header != None)
                    {
                        return Err(format!(
                            "Client '{}' is using some filtering option but filter is not defined",
                            client.name
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

    return Ok(config);
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
                    // Do basic verifications on configuration
                    match verify_config(v) {
                        Ok(vv) => return Ok(vv),
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

    /// Hold a package. The sender is woken only when the buffer was empty: otherwise it
    /// already waits for a head that arrived earlier, so this package cannot be due sooner.
    fn push(&self, ts: u128, data: Vec<u8>, arrival: Instant) {
        let mut st = self.state.lock().unwrap();
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
        self.state.lock().unwrap().heap.len()
    }

    /// No child will push again: the sender releases everything left, in order, and ends
    fn finish(&self) {
        self.state.lock().unwrap().finish = true;
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

/// The sender's loop: wait for due packages and hand each batch to `deliver` in order;
/// end once finish() was called and the buffer is empty. No periodic wake-up: it sleeps
/// until the head is due or a push into an empty buffer wakes it.
fn send_ordered(buffer: &OrderBuffer, hold: Duration, mut deliver: impl FnMut(Vec<Vec<u8>>)) {
    let mut st = buffer.state.lock().unwrap();
    loop {
        let batch = release(&mut st, Instant::now(), hold);
        if !batch.is_empty() {
            // Deliver without the lock, so children keep pushing meanwhile
            drop(st);
            deliver(batch);
            st = buffer.state.lock().unwrap();
            continue;
        }
        if st.finish {
            return;
        }
        st = match next_release(&st, hold) {
            Some(due) => {
                let wait = due.saturating_duration_since(Instant::now());
                buffer.wake.wait_timeout(st, wait).unwrap().0
            }
            None => buffer.wake.wait(st).unwrap(),
        };
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
    let mut clients = new_targets(id, &config, &filters);
    let mut cursor: usize = 0;
    send_ordered(buffer, hold, |batch| {
        let (mut outgoing, mut dropped, mut deleted) = (0, 0, 0);
        for package in &batch {
            deliver(
                id,
                &filters.root,
                &config,
                &mut clients,
                &mut cursor,
                package,
                &mut outgoing,
                &mut dropped,
                &mut deleted,
            );
        }
        report(
            worker_slot(&stats, id),
            &clients,
            0,
            outgoing,
            dropped,
            deleted,
        );
    });
}

/// One link per target, in configuration order, each with its compiled filter; every
/// target gets a first connect attempt now
fn new_targets(id: u16, config: &Config, filters: &Filters) -> Vec<RedisLink> {
    let mut clients: Vec<RedisLink> = config
        .clients
        .iter()
        .zip(&filters.clients)
        .map(|(client, filter)| RedisLink {
            config: client.clone(),
            link: None,
            retry_at: None,
            backoff: TARGET_BACKOFF_MIN,
            sleeping_from: 0,
            packages: 0,
            lastcheck: 0,
            filter: filter.clone(),
            request: Vec::new(),
        })
        .collect();
    for client in clients.iter_mut() {
        ensure_link(id, client);
    }
    clients
}

/// Whether the target has a link, connecting it first when it has none and its retry time
/// has come; a failed attempt waits a second before the next one
fn ensure_link(id: u16, client: &mut RedisLink) -> bool {
    if client.link.is_some() {
        return true;
    }
    let now = Instant::now();
    if client.retry_at.is_some_and(|at| now < at) {
        return false;
    }
    match redis_connect(id, &Endpoint::target(&client.config), true) {
        Ok(link) => {
            client.link = Some(link);
            client.retry_at = None;
            true
        }
        Err(e) => {
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
            mark_down(client);
            false
        }
    }
}

/// No link until the backoff has passed; the next failure waits twice as long, up to
/// TARGET_BACKOFF_MAX (plan 2.2). A command that succeeds resets it.
fn mark_down(client: &mut RedisLink) {
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
    let mut clients = new_targets(id, &config, &filters);
    let source_endpoint = Endpoint::source(&config);

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
                    let mut incoming: u64 = 0;
                    let mut outgoing: u64 = 0;
                    let mut dropped: u64 = 0;
                    let mut deleted: u64 = 0;
                    let mut cursor: usize = 0; // spreader: next target to try
                                               // The BLPOP request never changes: encoded once per connection (M3-o)
                    let blpop = blpop_cmd(&config.channel).get_packed_command();
                    let mut lasttime = get_current_time();
                    while keepworking {
                        // Check if we should save statistics
                        if (get_current_time() - 1) > lasttime {
                            report(
                                worker_slot(&stats, id),
                                &clients,
                                incoming,
                                outgoing,
                                dropped,
                                deleted,
                            );

                            // Reset status
                            incoming = 0;
                            outgoing = 0;
                            dropped = 0;
                            deleted = 0;
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
                                    refresh(id, &mut clients, &mut deleted);
                                }
                                Ok(redis::Value::Array(data)) => {
                                    // This is the expected data
                                    // println!("Bulk(Vec<Value>)");

                                    // Send to all clients
                                    incoming += 1;

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
                                                &mut outgoing,
                                                &mut dropped,
                                                &mut deleted,
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
                    report(
                        worker_slot(&stats, id),
                        &clients,
                        incoming,
                        outgoing,
                        dropped,
                        deleted,
                    );
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
) {
    // Counts of every child since the last report
    let (incoming, outgoing, dropped, deleted) = take_counts(stats);

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

    // Write statistics
    if let Some(status) = statusfile {
        let stat = json!({
            "date": get_current_time(),
            "in": (incoming as f64) / diff,
            "out": (outgoing as f64) / diff,
            "drop": (dropped as f64) / diff,
            "deleted": (deleted as f64) / diff,
            "total_in": incoming,
            "total_out": outgoing,
            "total_drop": dropped,
            "total_deleted": deleted,
        });
        match fs::write(status, stat.to_string()) {
            Ok(_) => (),
            Err(e) => print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_RED,
                0,
                "Unable to write STATUS at {}: {}",
                status,
                e
            ),
        }
    }

    // Show tail
    print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD, "");
}

/// A worker's counts since its last report and the stuck state of its targets
fn report(
    slot: &WorkerStats,
    clients: &[RedisLink],
    incoming: u64,
    outgoing: u64,
    dropped: u64,
    deleted: u64,
) {
    slot.add(incoming, outgoing, dropped, deleted);
    for (flag, client) in slot.stuck.iter().zip(clients) {
        flag.store(client.sleeping_from > 0, Ordering::Relaxed);
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
    if !is_client {
        return client
            .get_connection()
            .map_err(|e| format!("Couldn't connect to Redis Server: {}", e));
    }
    // A target that drops packets instead of refusing would hold the thread for the
    // kernel's connect timeout (about 2 min), and one that stops answering would hold it
    // for ever: bounded here, so the other targets keep receiving (M3R-12 X1b)
    let link = client
        .get_connection_with_timeout(TARGET_CONNECT_TIMEOUT)
        .map_err(|e| format!("Couldn't connect to Redis Server: {}", e))?;
    link.set_read_timeout(Some(TARGET_IO_TIMEOUT))
        .and_then(|_| link.set_write_timeout(Some(TARGET_IO_TIMEOUT)))
        .map_err(|e| format!("Couldn't set the timeouts: {}", e))?;
    Ok(link)
}

/// BLPOP with a one second timeout, sent as an integer as redis 0.19 did:
/// Commands::blpop takes an f64 and sends "1.0", which servers before 6.0 reject
fn blpop_cmd(channel: &str) -> redis::Cmd {
    let mut cmd = redis::cmd("BLPOP");
    cmd.arg(channel).arg(1);
    cmd
}

fn send_to_client(client: &mut RedisLink, data: &[u8]) -> Result<bool, String> {
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
            Ok(true)
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

fn can_check_queue(
    timelimit: Option<u64>,
    checklimit: Option<u64>,
    packages: u64,
    lastcheck: u64,
) -> bool {
    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        0,
        "can_check_queue(): timelimit={:?}  checklimit={:?}   packages={}   lastcheck:{}",
        timelimit,
        checklimit,
        packages,
        lastcheck
    );

    // No config
    if (timelimit == None) && (checklimit == None) {
        // No configuration set, using default CHECK SECONDS
        #[cfg(feature = "debug")]
        print_debug!(
            PROGRAM_NAME,
            stdout(),
            COLOR_WHITE,
            0,
            "can_check_queue(): No configuration set, using DEFAULT_CHECK_SECONDS={} -> {:?}",
            DEFAULT_CHECK_SECONDS,
            (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time()
        );
        return (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time();
    } else {
        // Check by timelimit
        match timelimit {
            None => (),
            Some(ts) => {
                if (lastcheck + ts) < get_current_time() {
                    #[cfg(feature = "debug")]
                    print_debug!(
                        PROGRAM_NAME,
                        stdout(),
                        COLOR_GREEN,
                        0,
                        "can_check_queue(): timelimit=true"
                    );
                    return true;
                } else {
                    #[cfg(feature = "debug")]
                    print_debug!(
                        PROGRAM_NAME,
                        stdout(),
                        COLOR_RED,
                        0,
                        "can_check_queue(): timelimit=false"
                    );
                }
            }
        }
        // Check by checklimit
        match checklimit {
            None => (),
            Some(_) => {
                if packages == 0 {
                    #[cfg(feature = "debug")]
                    print_debug!(
                        PROGRAM_NAME,
                        stdout(),
                        COLOR_GREEN,
                        0,
                        "can_check_queue(): packages=true"
                    );
                    return true;
                } else {
                    #[cfg(feature = "debug")]
                    print_debug!(
                        PROGRAM_NAME,
                        stdout(),
                        COLOR_RED,
                        0,
                        "can_check_queue(): packages=false"
                    );
                }
            }
        }
    }

    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_RED,
        0,
        "can_check_queue(): default=false"
    );

    return false;
}

fn can_send(id: u16, client: &mut RedisLink, deleted: &mut u64) -> Result<bool, String> {
    // Check if we can check queue
    if can_check_queue(
        client.config.timelimit,
        client.config.checklimit,
        client.packages,
        client.lastcheck,
    ) {
        // Reset timers
        client.lastcheck = get_current_time();
        match client.config.checklimit {
            None => client.packages = 0,
            Some(v) => client.packages = v,
        }

        // Let's check the queue
        let result: redis::RedisResult<i32> = link(&mut client.link)?.llen(&client.config.channel);
        match result {
            Ok(len) => {
                if client.config.hardlimit != None {
                    if client.sleeping_from == 0 {
                        // The client is not sleeping
                        if (len as u64) >= client.config.hardlimit.unwrap() {
                            if client.config.deleteblock == None {
                                // We lock the client
                                client.sleeping_from = get_current_time();
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
                                // Deleteblock in action
                                let mut actual_len: u64 = len as u64;
                                while actual_len >= client.config.hardlimit.unwrap() {
                                    // Trim elements from the queue
                                    let result: redis::RedisResult<redis::Value> =
                                        link(&mut client.link)?.ltrim(
                                            &client.config.channel,
                                            client.config.deleteblock.unwrap() as isize,
                                            MAX_QUEUE_SIZE,
                                        );
                                    match result {
                                        Ok(_) => *deleted += client.config.deleteblock.unwrap(),
                                        Err(e) => {
                                            link_broken(client, &e);
                                            return Err(format!("error in deleteblock while deleting block with {} elements: {}", client.config.deleteblock.unwrap(), e));
                                        }
                                    }

                                    // Read len again
                                    let result: redis::RedisResult<i32> =
                                        link(&mut client.link)?.llen(&client.config.channel);
                                    match result {
                                        Ok(len) => actual_len = len as u64,
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
                        if (len as u64) < client.config.softlimit.unwrap() {
                            // We lock the client
                            client.sleeping_from = 0;
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
                    if client.sleeping_from == 0 {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   true", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    } else {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   false", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    }
                }

                // If not stuck, can keep sending
                return Ok(client.sleeping_from == 0);
            }
            Err(e) => {
                link_broken(client, &e);
                return Err(format!("error requesting the length to the channel: {}", e));
            }
        };
    } else {
        // Count down packages
        client.packages -= 1;

        #[cfg(feature = "debug")]
        {
            if client.sleeping_from == 0 {
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
        return Ok(client.sleeping_from == 0);
    }
}

fn send(
    id: u16,
    client: &mut RedisLink,
    dirty_bdata: &[u8],
    deleted: &mut u64,
) -> Result<bool, String> {
    let bdata = match &client.filter {
        Some(f) => match f.apply(dirty_bdata) {
            Some(b) => b,
            None => return Ok(false),
        },
        None => Cow::Borrowed(dirty_bdata),
    };
    // If we can send to this queue
    match can_send(id, client, deleted) {
        // Allowed to send
        Ok(true) => match send_to_client(client, &bdata) {
            Ok(true) => Ok(true),
            Ok(false) => Ok(false),
            Err(e) => Err(format!("error while sending to the client: {}", e)),
        },

        // Not allowed to send
        Ok(false) => Ok(false),

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
    outgoing: &mut u64,
    dropped: &mut u64,
    deleted: &mut u64,
) {
    let bdata = match root_filter {
        Some(f) => f.apply(package),
        None => Some(Cow::Borrowed(package)),
    };
    let mut attempt = |client: &mut RedisLink, bdata: &[u8]| {
        if !ensure_link(id, client) {
            return false;
        }
        match send(id, client, bdata, deleted) {
            Ok(sent) => sent,
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
        *outgoing += 1;
    } else {
        *dropped += 1;
    }
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

/// The idle path: refresh the limits state of every reachable target (1.0.8 behaviour,
/// M3F-7 is stage 4); a failing target is handled alone, the worker goes on
fn refresh(id: u16, clients: &mut [RedisLink], deleted: &mut u64) {
    for client in clients.iter_mut() {
        if !ensure_link(id, client) {
            continue;
        }
        // Only the refresh matters, not whether it could send
        if let Err(e) = can_send(id, client, deleted) {
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

#[cfg(test)]
mod tests;
