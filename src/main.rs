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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::{cmp::Reverse, collections::BinaryHeap};
use thread_tryjoin::TryJoinHandle;

// use std::env;
use redis::Commands;
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
    config: ClientConfig,    // Client configuration
    link: redis::Connection, // Client opened link to Redis
    sleeping_from: u64,      // If queue is stuck, when did it happened
    packages: u64,           // Packages we have seen from last check
    lastcheck: u64,          // When was the last check of queue's size (time limit)
    filter: Option<Filter>,  // Client filter, compiled at start
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
                    let is_ordering_regex: bool;
                    let regex_str: &str;
                    if let Some(r) = inconfig.ordering.clone() {
                        regex_str = &r;
                        let filter_temp = Regex::new(&regex_str);
                        match filter_temp {
                            Ok(f) => {
                                ordering_regex = Some(f);
                                is_ordering_regex = true;
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
                                is_ordering_regex = false;
                                error = true;
                            }
                        }
                    } else {
                        ordering_regex = None;
                        is_ordering_regex = false;
                    }

                    if !error {
                        // Set handler
                        let (keepworking_tx, keepworking_rx): (Sender<bool>, Receiver<bool>) =
                            mpsc::channel();
                        ctrlc::set_handler(move || {
                            print_debug!(
                                PROGRAM_NAME,
                                stdout(),
                                COLOR_GREEN,
                                0,
                                "User requested to exit!"
                            );
                            keepworking_tx.send(false).unwrap();
                        })
                        .expect("Error setting Ctrl-C handler");

                        // Let communicate with children to end
                        let (queue_tx, queue_rx): (
                            Sender<(u16, Option<u128>, Option<Vec<u8>>)>,
                            Receiver<(u16, Option<u128>, Option<Vec<u8>>)>,
                        ) = mpsc::channel();
                        let (queue_working_tx, queue_working_rx): (Sender<bool>, Receiver<bool>) =
                            mpsc::channel();
                        // Counters of every child, read by this loop (no messages)
                        let stats = stats_slots(inconfig.children, inconfig.clients.len());
                        let Links {
                            answers: queues_channels,
                            stops: keepworkings,
                            children: children_channels,
                        } = links(inconfig.children);

                        // Prepare list of handles
                        let mut handles: Vec<thread::JoinHandle<_>> = Vec::new();
                        let queue_len = Arc::new(AtomicUsize::new(0));

                        // Spawn a queue manage
                        let queue_config = inconfig.clone();
                        let queue_len_w = queue_len.clone();
                        let queue_handler = thread::spawn(move || {
                            queuer(
                                is_ordering_regex,
                                queue_config.clone(),
                                queue_working_rx,
                                queue_rx,
                                queues_channels,
                                queue_len_w,
                            )
                        });

                        // Spawn a number of threads and collect their join handles; each child
                        // gets the ends paired with its id by links()
                        for (id, qrx, rx) in children_channels {
                            // Clone config and execute thread
                            let child_stats = stats.clone();
                            let qtx = queue_tx.clone();
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
                                    rx,
                                    qtx,
                                    &qrx,
                                    child_config,
                                    fr,
                                );
                            });
                            handles.push(handle);

                            // Do not rush
                            thread::sleep(Duration::from_millis(1));
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
                            // Check if there is some message for us (from CTRL+C)
                            match keepworking_rx.try_recv() {
                                Ok(true) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading message from CTRL+C, got an unexpected message!"),
                                Ok(false) => keepworking = false,
                                Err(_) => (),
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
                                    // Check if queuer has finished
                                    if queue_handler.is_finished() {
                                        // Some child already died, show message and leave
                                        print_debug!(
                                            PROGRAM_NAME,
                                            stdout(),
                                            COLOR_YELLOW,
                                            0,
                                            "Queuer is closed abruptly, closing everything!"
                                        );
                                        break;
                                    }

                                    // Check if we should show statistics
                                    if (get_current_time_with_ms() - (STATISTICS_SECONDS * 1000))
                                        > lasttime
                                    {
                                        // Counts of every child since the last report
                                        let (incoming, outgoing, dropped, deleted) =
                                            take_counts(&stats);

                                        // Show statistics
                                        let diff: f64 = ((get_current_time_with_ms() - lasttime)
                                            as f64)
                                            / 1000.0;
                                        print_debug!(
                                            PROGRAM_NAME,
                                            stdout(),
                                            COLOR_BLUE,
                                            COLOR_NOTAIL,
                                            "{} - ",
                                            inconfig.name
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
                                        if inconfig.ordering != None {
                                            print_debug!(
                                                PROGRAM_NAME,
                                                stdout(),
                                                COLOR_GREEN,
                                                COLOR_NOHEAD_NOTAIL,
                                                "Queue: {} regs",
                                                queue_len.load(Ordering::Relaxed)
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
                                        let stucks: Vec<String> =
                                            stuck_clients(&stats, inconfig.clients.len())
                                                .into_iter()
                                                .map(|c| {
                                                    let client = &inconfig.clients[c];
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
                                        if let Some(status) = &statusfile {
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
                                        print_debug!(
                                            PROGRAM_NAME,
                                            stdout(),
                                            COLOR_WHITE,
                                            COLOR_NOHEAD,
                                            ""
                                        );
                                        lasttime = get_current_time_with_ms();
                                    }

                                    // Sleep a sec
                                    thread::sleep(Duration::from_millis(1000));
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

                        // Tell queuer to serve all data that is left
                        queue_working_tx.send(true).unwrap();

                        // Try to tell all children to finish their job
                        for keep in keepworkings {
                            match keep.send(false) {
                                Ok(_) => (),
                                Err(_) => (),
                            }
                        }

                        // Wait for children to close
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Closing...");
                        thread::sleep(Duration::from_millis(2000));

                        // Wait for all children to close
                        let mut working = 1;
                        let mut joined: Vec<u16> = Vec::new();
                        while working > 0 {
                            // Not working at this moment
                            working = 0;

                            let mut counter: u16 = 0;
                            for h in &handles {
                                // Check if this handle was already joined
                                if !joined.contains(&counter) {
                                    // Try to join it
                                    match h.try_timed_join(Duration::from_millis(1)) {
                                        // It joined, it is dead!
                                        Ok(_) => {
                                            joined.push(counter);
                                        }
                                        // Didn't join, it is alive
                                        Err(_) => working += 1,
                                    }
                                }
                                counter += 1;
                            }

                            // If somebody working, show up
                            if working > 0 {
                                print_debug!(
                                    PROGRAM_NAME,
                                    stdout(),
                                    COLOR_YELLOW,
                                    0,
                                    "There are {} threads left!",
                                    working
                                );
                                thread::sleep(Duration::from_millis(1000));
                            }
                        }

                        // Tell the Queue to close
                        queue_working_tx.send(false).unwrap();
                        queue_handler.join().unwrap();

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

/// Manage ordered packages in a centralized way
fn queuer(
    is_ordering_regex: bool,
    config: Config,
    keepworking_rx: Receiver<bool>,
    children_rx: Receiver<(u16, Option<u128>, Option<Vec<u8>>)>,
    children_tx: Vec<Sender<Vec<Vec<u8>>>>,
    queue_len: Arc<AtomicUsize>,
) {
    #[cfg(feature = "debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Starts");

    // Prepare main variable
    let mut keepworking = true;
    let mut dumpall: bool;
    if is_ordering_regex {
        dumpall = false;
    } else {
        dumpall = true;
    }

    // Prepare sorted list
    let mut ordered_packages: BinaryHeap<Reverse<(u128, (u64, Vec<u8>))>> = BinaryHeap::new();

    // Prepare the retention data
    while keepworking {
        // Check if some worker requested data
        let request = children_rx.recv_timeout(Duration::from_millis(1));

        // Read main's order before answering: main sends it before telling the children to
        // stop, so a request sent after a child's stop order is always answered with dumpall
        match keepworking_rx.try_recv() {
            Ok(true) => {
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_BLUE,
                    0,
                    "Queue: Prepared to die"
                );
                dumpall = true;
            }
            Ok(false) => keepworking = false,
            Err(_) => (),
        }

        match request {
            Ok((id, ts, package)) => {
                #[cfg(feature = "debug")]
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_BLUE,
                    0,
                    "Queue: Got request from {}: {:?}",
                    id,
                    package
                );

                let mut list: Vec<Vec<u8>>;
                if dumpall {
                    list = Vec::new();
                    while let Some(Reverse(package)) = ordered_packages.pop() {
                        list.push(package.1 .1);
                    }
                    if let Some(p) = package {
                        list.push(p);
                    }
                } else {
                    list = match_ordering(
                        ts,
                        config.ordering_buffer_time,
                        package,
                        &mut ordered_packages,
                    );
                }

                #[cfg(feature = "debug")]
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_BLUE,
                    0,
                    "Queue: Send answer to {}: {}",
                    id,
                    list.len()
                );

                // Publish the size of the queue for the statistics line (before the answer, so
                // a child that got its answer sees the size that goes with it)
                queue_len.store(ordered_packages.len(), Ordering::Relaxed);

                // Answer to the child
                answer_child(&children_tx, id, list);
            }
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => {
                // Unreachable while main holds queue_tx; pause so it cannot spin
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[cfg(feature = "debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Ends");
}

/// Channel ends between main, the queuer and the children. The queuer answers child `id` on
/// `answers[id]` and main stops it on `stops[id]`; `children` yields each child id with the
/// receiving ends of those same channels, so a child reads only its own answers (M2R-2)
struct Links {
    answers: Vec<Sender<Vec<Vec<u8>>>>,
    stops: Vec<Sender<bool>>,
    children: Vec<(u16, Receiver<Vec<Vec<u8>>>, Receiver<bool>)>,
}

fn links(children: u16) -> Links {
    let mut links = Links {
        answers: Vec::new(),
        stops: Vec::new(),
        children: Vec::new(),
    };
    for id in 0..children {
        let (qtx, qrx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        links.answers.push(qtx);
        links.stops.push(tx);
        links.children.push((id, qrx, rx));
    }
    links
}

/// The queuer's answer to the request of child `id`
fn answer_child(answers: &[Sender<Vec<Vec<u8>>>], id: u16, list: Vec<Vec<u8>>) {
    answers[id as usize].send(list).unwrap();
}

/// Manage the full process from a child
fn child(
    id: u16,
    ordering_regex: Option<Regex>,
    ordering_limit: Option<usize>,
    stats: Arc<Vec<WorkerStats>>,
    rx: Receiver<bool>,
    qtx: Sender<(u16, Option<u128>, Option<Vec<u8>>)>,
    qrx: &Receiver<Vec<Vec<u8>>>,
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

    // Prepare the retention data
    while keepworking {
        // Check if our father wants us to finish
        match rx.try_recv() {
            Ok(true) => print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_RED,
                0,
                "Error while reading message from Parent, got an unexpected message!"
            ),
            Ok(false) => {
                #[cfg(feature = "debug")]
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_WHITE,
                    0,
                    "{}: I was told to close!",
                    id
                );
                request_finish = true;
            }
            Err(_) => (),
        }

        // Connect to source
        let mut source: redis::Connection;
        let mut error = false;
        match redis_connect(
            id,
            config.ssl,
            config.hostname.clone(),
            config.port,
            config.password.clone(),
            false,
        ) {
            Ok(link) => {
                source = link;

                // Connect to targets
                let mut clients: Vec<RedisLink> = Vec::new();
                for (client, filter) in config.clients.iter().zip(&filters.clients) {
                    match redis_connect(
                        id,
                        client.ssl,
                        client.hostname.clone(),
                        client.port,
                        client.password.clone(),
                        true,
                    ) {
                        Ok(link) => clients.push(RedisLink {
                            config: client.clone(),
                            link: link,
                            sleeping_from: 0,
                            packages: 0,
                            lastcheck: 0,
                            filter: filter.clone(),
                        }),
                        Err(e) => {
                            print_debug!(
                                PROGRAM_NAME,
                                stderr(),
                                COLOR_RED,
                                0,
                                "Error while connecting to target Redis Server: {}",
                                e
                            );
                            error = true;
                            break;
                        }
                    }
                }

                // No error until here, keep going
                if !error {
                    // Keep working while allowed
                    let mut incoming: u64 = 0;
                    let mut outgoing: u64 = 0;
                    let mut dropped: u64 = 0;
                    let mut deleted: u64 = 0;
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
                            let item: redis::RedisResult<redis::Value> =
                                blpop_cmd(&config.channel).query(&mut source);
                            match item {
                                Ok(redis::Value::Nil) => {
                                    // {println!("Nil")},
                                    // Process no data
                                    match process_package(
                                        id,
                                        &ordering_regex,
                                        ordering_limit,
                                        &qtx,
                                        &qrx,
                                        &filters.root,
                                        &config,
                                        &mut clients,
                                        None,
                                        &mut outgoing,
                                        &mut dropped,
                                        &mut deleted,
                                    ) {
                                        Ok(_) => (),
                                        Err(e) => {
                                            error = true;
                                            print_debug!(
                                                PROGRAM_NAME,
                                                stderr(),
                                                COLOR_RED,
                                                0,
                                                "{}: process_package() has failed: {}",
                                                id,
                                                e
                                            );
                                        }
                                    }
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
                                        match process_package(
                                            id,
                                            &ordering_regex,
                                            ordering_limit,
                                            &qtx,
                                            &qrx,
                                            &filters.root,
                                            &config,
                                            &mut clients,
                                            Some(raw_bdata),
                                            &mut outgoing,
                                            &mut dropped,
                                            &mut deleted,
                                        ) {
                                            Ok(_) => (),
                                            Err(e) => {
                                                print_debug!(
                                                    PROGRAM_NAME,
                                                    stderr(),
                                                    COLOR_RED,
                                                    0,
                                                    "{}: Couldn't process package: {}",
                                                    id,
                                                    e
                                                );
                                            }
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
                            match rx.try_recv() {
                                Ok(true) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading message from Parent, got an unexpected message!"),
                                Ok(false) => {

                                    // We are requested to finish
                                    #[cfg(feature="debug")]
                                    print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "{}: I was told to close!", id);
                                    request_finish = true;

                                },
                                Err(_) => (),
                            }
                        } else {
                            // Get data left in the queue
                            let jobdone;
                            match process_package(
                                id,
                                &ordering_regex,
                                ordering_limit,
                                &qtx,
                                &qrx,
                                &filters.root,
                                &config,
                                &mut clients,
                                None,
                                &mut outgoing,
                                &mut dropped,
                                &mut deleted,
                            ) {
                                Ok(v) => jobdone = v,
                                Err(e) => {
                                    jobdone = false;
                                    error = true;
                                    print_debug!(
                                        PROGRAM_NAME,
                                        stderr(),
                                        COLOR_RED,
                                        0,
                                        "{}: process_package() has failed: {}",
                                        id,
                                        e
                                    );
                                }
                            }

                            // Decide about keep working (if requested to finish and nothing to do)
                            keepworking = jobdone && !error;
                        }

                        // Error found while processing
                        if error {
                            thread::sleep(Duration::from_millis(1000));
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
                } else {
                    thread::sleep(Duration::from_millis(1000));
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
                thread::sleep(Duration::from_millis(1000));
            }
        }

        // Check if our father wants us to finish
        match rx.try_recv() {
            Ok(true) => print_debug!(
                PROGRAM_NAME,
                stderr(),
                COLOR_RED,
                0,
                "Error while reading message from Parent, got an unexpected message!"
            ),
            Ok(false) => {
                // We are requested to finish
                #[cfg(feature = "debug")]
                print_debug!(
                    PROGRAM_NAME,
                    stdout(),
                    COLOR_WHITE,
                    0,
                    "{}: I was told to close!",
                    id
                );
                keepworking = false;
            }
            Err(_) => (),
        }
    }

    // Sleep a sec
    thread::sleep(Duration::from_millis(1500));

    #[cfg(feature = "debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Ends", id);
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
    ssl: Option<bool>,
    hostname: String,
    port: u16,
    password: String,
    _is_client: bool,
) -> Result<redis::Connection, String> {
    // If Redis server needs secure connection
    let mut uri_scheme = "redis";
    if let Some(inssl) = ssl {
        if inssl {
            uri_scheme = "rediss";
        }
    }

    // Prepare URL
    let redis_conn_url = format!("{}://:{}@{}:{}", uri_scheme, password, hostname, port);

    #[cfg(feature = "debug")]
    {
        let password_debug: String;
        if password.len() > 3 {
            password_debug = format!("{}***", &password[..3]);
        } else {
            password_debug = String::from("***");
        }
        let redis_conn_url_debug =
            format!("{}://:{}@{}:{}", uri_scheme, password_debug, hostname, port);
        if _is_client {
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_CYAN,
                0,
                "Child {}: Connecting to client: {}",
                _id,
                redis_conn_url_debug
            );
        } else {
            print_debug!(
                PROGRAM_NAME,
                stdout(),
                COLOR_BLUE,
                0,
                "Child {}: Connecting: {}",
                _id,
                redis_conn_url_debug
            );
        }
    }

    match redis::Client::open(redis_conn_url)
        .expect("Invalid connection URL")
        .get_connection()
    {
        Ok(c) => return Ok(c),
        Err(e) => return Err(format!("Couldn't connect to Redis Server: {}", e)),
    }
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

    let result: redis::RedisResult<i32> = client.link.rpush(channel, data);
    match result {
        Ok(_) => return Ok(true),
        Err(e) => return Err(format!("couldn't push to channel: {}", e)),
    };
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
        let result: redis::RedisResult<i32> = client.link.llen(&client.config.channel);
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
                                        client.link.ltrim(
                                            &client.config.channel,
                                            client.config.deleteblock.unwrap() as isize,
                                            MAX_QUEUE_SIZE,
                                        );
                                    match result{
                                        Ok(_) => *deleted += client.config.deleteblock.unwrap(),
                                        Err(e) => return Err(format!("error in deleteblock while deleting block with {} elements: {}", client.config.deleteblock.unwrap(), e)),
                                    }

                                    // Read len again
                                    let result: redis::RedisResult<i32> =
                                        client.link.llen(&client.config.channel);
                                    match result{
                                        Ok(len) => actual_len = len as u64,
                                        Err(e) => return Err(format!("error in deleteblock while requesting the length to the channel: {}", e)),
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
            Err(e) => return Err(format!("error requesting the length to the channel: {}", e)),
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

fn match_ordering(
    ts: Option<u128>,
    time: Option<u64>,
    bdata: Option<Vec<u8>>,
    buffer: &mut BinaryHeap<Reverse<(u128, (u64, Vec<u8>))>>,
) -> Vec<Vec<u8>> {
    let mut list: Vec<Vec<u8>> = Vec::new();

    // If we got a package
    if let Some(data) = bdata {
        if let Some(v) = ts {
            buffer.push(Reverse((v, (get_current_time(), data))));
        } else {
            // No filter available, just send it
            list.push(data);
        }
    }

    // Attach all packages from the buffer that should be sent already
    let control;
    if let Some(timer) = time {
        control = get_current_time() - timer;
    } else {
        // Make sure control is in the future (1s for security + 1s for strict < on comparison)
        control = get_current_time() + 2;
    }
    while let Some(Reverse(package)) = buffer.peek() {
        if package.1 .0 < control {
            let Reverse(p) = buffer.pop().unwrap();
            list.push(p.1 .1);
        } else {
            break;
        }
    }

    // #[cfg(feature="debug")]
    // {
    //     println!("BUFFER: {:?}", buffer);
    //     println!("LIST:   {:?}", list);
    // }

    // Return the list back
    return list;
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

fn process_package(
    id: u16,
    ordering_regex: &Option<Regex>,
    ordering_limit: Option<usize>,
    qtx: &Sender<(u16, Option<u128>, Option<Vec<u8>>)>,
    qrx: &Receiver<Vec<Vec<u8>>>,
    root_filter: &Option<Filter>,
    config: &Config,
    clients: &mut Vec<RedisLink>,
    package: Option<Vec<u8>>,
    outgoing: &mut u64,
    dropped: &mut u64,
    deleted: &mut u64,
) -> Result<bool, String> {
    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        0,
        "{}: Start process_package(): data={:?}",
        id,
        package
    );

    // Check if we got a package
    if let Some(data) = package {
        let ts: Option<u128>;

        // Process regex
        if let Some(re) = ordering_regex {
            match ordering_ts(re, ordering_limit, &data) {
                Ok(n) => ts = Some(n),
                Err(warning) => {
                    // No TS information, just send it
                    print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, 0, "{}", warning);
                    ts = None;
                }
            }
        } else {
            // No filter available, just send it
            ts = None;
        }

        // Send it to queuer (moved, not copied)
        qtx.send((id, ts, Some(data))).unwrap();
    } else {
        // Just say we didn't get anything
        qtx.send((id, None, None)).unwrap();
    }

    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        0,
        "{}: Sent request to Queuer process_package()",
        id
    );

    // Check if there is some work to be done
    let list: Vec<Vec<u8>> = qrx.recv().unwrap();

    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        0,
        "{}: Got answer from Queuer process_package(): {}",
        id,
        list.len()
    );

    // Check if we got packages to send
    let jobdone: bool;
    if list.len() > 0 {
        for package in list {
            // Ready to send data
            let total_clients = clients.len();
            let mut errors = 0;

            let filtered = match root_filter {
                Some(f) => f.apply(&package),
                None => Some(Cow::Borrowed(&package[..])),
            };
            match filtered {
                None => errors = total_clients,
                Some(bdata) => {
                    if config.mode == "replicant" {
                        // Send data to all clients
                        for client in clients.iter_mut() {
                            // If we can send to this queu
                            match send(id, client, &bdata, deleted) {
                                // Data sent
                                Ok(true) => (),

                                // Not sent
                                Ok(false) => {
                                    errors += 1;
                                }

                                // There was an error
                                Err(e) => {
                                    // There was an error
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
                                    errors += 1;
                                }
                            }
                        }
                    } else {
                        // Send data to next client
                        let mut done = false;

                        // We will go throught all clients until data is
                        // sent or all clients have failed
                        while (!done) && (errors < total_clients) {
                            // Try to send to this client
                            let client = &mut clients[0];

                            // If we can send to this queu
                            match send(id, client, &bdata, deleted) {
                                // Data sent
                                Ok(true) => done = true,

                                // Not sent
                                Ok(false) => {
                                    errors += 1;
                                }

                                // There was an error
                                Err(e) => {
                                    // There was an error
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
                                    errors += 1;
                                }
                            }

                            // Rotate
                            if total_clients > 1 {
                                for i in 0..(total_clients - 1) {
                                    clients.swap(i, i + 1);
                                }
                            }

                            // Leave if we are done
                            if done {
                                break;
                            }
                        }
                    }
                }
            }

            // If all clients have failed, drop the package and set error
            if errors == total_clients {
                // No sent at all
                *dropped += 1;
            } else {
                // The package was sent at least to 1 node
                *outgoing += 1;
            }
        }

        jobdone = true;
    } else {
        // Refresh clients status
        for client in clients.iter_mut() {
            match can_send(id, client, deleted) {
                Ok(_) => (), // We do not care if it can send or not (just wanted to refresh client information)
                Err(e) => {
                    #[cfg(feature = "debug")]
                    print_debug!(
                        PROGRAM_NAME,
                        stdout(),
                        COLOR_CYAN,
                        0,
                        "{}: End unexpected process_package()",
                        id
                    );

                    return Err(format!(
                        "couldn't check queue for {}: {}",
                        client.config.channel, e
                    ));
                }
            }
        }
        jobdone = false;
    }

    #[cfg(feature = "debug")]
    print_debug!(
        PROGRAM_NAME,
        stdout(),
        COLOR_CYAN,
        0,
        "{}: End process_package() - JOBDONE: {:?}",
        id,
        jobdone
    );

    // Send if we did or didn't do the job
    return Ok(jobdone);
}

#[cfg(test)]
mod tests;
