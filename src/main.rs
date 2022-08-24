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

// use std::mem;
use std::thread;
use std::process;
use std::cmp;
use std::{cmp::Reverse, collections::BinaryHeap};
use std::str::from_utf8;
use thread_tryjoin::TryJoinHandle;
use std::time::Duration;
use std::io::{stdout, stderr, Write};
use serde::{Serialize, Deserialize};
use std::sync::mpsc::{Sender, Receiver};
use std::sync::mpsc;
use regex::Regex;
use serde_json::json;

// use std::env;
use std::fs;
use std::path::Path;
use redis::Commands;
use dict::{ Dict, DictIface };

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
    filter_limit: Option<usize>,
    filter_replace: Option<String>,
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
            filter_limit: self.filter_limit,
            filter_replace: self.filter_replace.clone(),
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
    filter_limit: Option<usize>,
    filter_replace: Option<String>,
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
            filter_limit: self.filter_limit,
            filter_replace: self.filter_replace.clone(),
            ordering: self.ordering.clone(),
            ordering_buffer_time: self.ordering_buffer_time,
            ordering_limit: self.ordering_limit,
            clients: self.clients.clone(),
        }
    }
}

/// Keep track of clients we are connected to
struct RedisLink {
    config: ClientConfig,       // Client configuration
    link: redis::Connection,    // Client opened link to Redis
    sleeping_from: u64,         // If queue is stuck, when did it happened
    packages: u64,              // Packages we have seen from last check
    lastcheck: u64,             // When was the last check of queue's size (time limit)
    regex: Option<Regex>,
}

#[allow(dead_code)]
enum MatchAnswer {
    Ok(bool),
    Box(String),
    Err(String),
}

/// Keep track of statistics per child
#[derive(Debug)]
struct Statistics {
    _id: u16,
    incoming: u64,
    outgoing: u64,
    dropped: u64,
    deleted: u64,
    stuck: Vec<(String, bool)>,
    finished: bool,
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
            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, COLOR_NOHEAD, "{} v{} ({})", PROGRAM_NAME, VERSION, BUILD_DATE);

            // Missing argument, show some help
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
            print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, COLOR_NOHEAD, "Usage: redismultiplexer <path_to_config.yaml> [debug]");
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
            print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "Documentation at: https://github.com/juanmitaboada/redismultiplexer");
            print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");

            // Answer with out version
            println!("");

        } else {

            // Welcome
            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{} v{} ({})", PROGRAM_NAME, VERSION, BUILD_DATE);

            #[cfg(feature="debug")]
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
            let mut config : Option<Config> = None;
            match get_config(final_path) {
                Ok(c) => config = Some(c),
                Err(e) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while processing configuration: {}", e),
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
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Unable to write PID at {}: {}", pid, e);
                            error = true;
                        },
                    }

                }

                // If not error until here
                if !error {

                    // Render main filter regex
                    let filter_regex: Option<Regex>;
                    let regex_str: &str;
                    if let Some(r) = inconfig.filter.clone() {
                        regex_str = &r;
                        let filter_temp = Regex::new(&regex_str);
                        match filter_temp {
                            Ok(f) => filter_regex = Some(f),
                            Err(e) => {
                                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Filter Regex '{}' doesn't compile: {}", r, e);
                                filter_regex = None;
                                error = true;
                            },
                        }
                    } else {
                        filter_regex = None;
                    }

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
                            },
                            Err(e) => {
                                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Ordering Regex '{}' doesn't compile: {}", r, e);
                                ordering_regex = None;
                                is_ordering_regex = false;
                                error = true;
                            },
                        }
                    } else {
                        ordering_regex = None;
                        is_ordering_regex = false;
                    }

                    if !error {

                        // Set handler
                        let (keepworking_tx, keepworking_rx): (Sender<bool>, Receiver<bool>) = mpsc::channel();
                        ctrlc::set_handler(move || {
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "User requested to exit!");
                            keepworking_tx.send(false).unwrap();
                        }).expect("Error setting Ctrl-C handler");

                        // Let communicate with children to end
                        let (queue_tx, queue_rx): (Sender<(u16, Option<u128>, Option<String>)>, Receiver<(u16, Option<u128>, Option<String>)>) = mpsc::channel();
                        let (queue_working_tx, queue_working_rx): (Sender<bool>, Receiver<bool>) = mpsc::channel();
                        let (children_tx, children_rx): (Sender<Statistics>, Receiver<Statistics>) = mpsc::channel();
                        let mut keepworkings: Vec<Sender<bool>> = Vec::new();
                        let mut queues_channels: Vec<Sender<Vec<String>>> = Vec::new();
                        let mut children_channels: Vec<(Receiver<Vec<String>>, Receiver<bool>)> = Vec::new();
                        for _ in 0..inconfig.children {

                            // Create queue channels for every child
                            let (qtx, qrx): (Sender<Vec<String>>, Receiver<Vec<String>>) = mpsc::channel();

                            // Create channels for every child
                            let (tx, rx): (Sender<bool>, Receiver<bool>) = mpsc::channel();

                            // Save channel information
                            children_channels.push((qrx, rx));

                            // Remember the channel so we can talk later with the child
                            queues_channels.push(qtx);
                            keepworkings.push(tx);
                        }

                        // Prepare list of handles
                        let mut handles: Vec<thread::JoinHandle<_>> = Vec::new();
                        let (queuer_stat_tx, queuer_stat_rx): (Sender<Option<usize>>, Receiver<Option<usize>>) = mpsc::channel();

                        // Spawn a queue manage
                        let queue_config = inconfig.clone();
                        let queue_handler = thread::spawn(move || {
                            queuer(is_ordering_regex, queue_config.clone(), queue_working_rx, queue_rx, queues_channels, queuer_stat_tx)
                        });

                        // Spawn a number of threads and collect their join handles
                        for id in 0..inconfig.children {

                            // Get channel for the child
                            let (qrx, rx) = children_channels.pop().unwrap();

                            // Clone config and execute thread
                            let tx = children_tx.clone();
                            let qtx = queue_tx.clone();
                            let child_config = inconfig.clone();
                            let fr = filter_regex.clone();
                            let or = ordering_regex.clone();
                            let handle = thread::spawn(move || {
                                child(id, or, inconfig.ordering_limit, tx, rx, qtx, &qrx, child_config, fr);
                            });
                            handles.push(handle);

                            // Do not rush
                            thread::sleep(Duration::from_millis(1));
                        }

                        // Show configuration
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "> {}:{} @ {} [{}, {} children]", inconfig.hostname, inconfig.port, inconfig.channel, inconfig.mode, inconfig.children);
                        for client in &inconfig.clients {
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "  - {}:{} @ {}  [timelimit={}, checklimit={}, softlimit={}, hardlimit={}]", client.hostname, client.port, client.channel, option2string!(client.timelimit), option2string!(client.checklimit), option2string!(client.softlimit), option2string!(client.hardlimit));
                        }

                        // Create dictionary of stuck clients
                        let mut stucked = Dict::<(String, bool)>::new();
                        for client in inconfig.clients {
                            stucked.add(client.name, (client.channel, false));
                        }

                        // Keep working while all children keep working
                        let mut lasttime = get_current_time_with_ms();
                        let mut incoming: u64 = 0;
                        let mut outgoing: u64 = 0;
                        let mut dropped: u64 = 0;
                        let mut deleted: u64 = 0;
                        let mut keepworking = true;
                        let mut queuer_working: bool = true;
                        let mut queuer_stat_size: usize = 0;
                        while keepworking {

                            // Check if there is some message for us (from CTRL+C)
                            match keepworking_rx.try_recv() {
                                Ok(true) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading message from CTRL+C, got an unexpected message!"),
                                Ok(false) => keepworking = false,
                                Err(_) => (),
                            }

                            // If nothind changed with children
                            if keepworking {

                                // While we get messages, keep reading
                                let mut got_message = true;
                                while got_message {

                                    got_message = false;
                                    let result_message = children_rx.try_recv();
                                    match result_message {
                                        Ok(msg) => {
                                            incoming += msg.incoming;
                                            outgoing += msg.outgoing;
                                            dropped += msg.dropped;
                                            deleted += msg.deleted;
                                            if msg.finished {
                                                keepworking = false;
                                            }
                                            for (name, newstatus) in msg.stuck {
                                                let (channel, _) = stucked.get(&name).unwrap();
                                                let value = (channel.clone(), newstatus);
                                                stucked.remove_key(&name).unwrap();
                                                stucked.add(name, value);
                                            }
                                            got_message=true
                                        },
                                        Err(_) => (),
                                    }

                                }

                                // Check if we can keep working
                                if keepworking {

                                    // Check if there is some message for us
                                    got_message = true;
                                    while got_message {
                                        match queuer_stat_rx.try_recv() {
                                            Ok(v) => {
                                                got_message = true;
                                                if let Some(size) = v {
                                                    queuer_stat_size=size;
                                                    queuer_working = true;
                                                } else {
                                                    queuer_working = false;
                                                }
                                            },
                                            Err(_) => got_message = false,
                                        }
                                    }

                                    // Check if queuer has finished
                                    if !queuer_working {
                                        // Some child already died, show message and leave
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "Queuer is closed abruptly, closing everything!");
                                        break;
                                    }


                                    // Check if we should show statistics
                                    if (get_current_time_with_ms() - (STATISTICS_SECONDS*1000))  > lasttime {
                                        // Show statistics
                                        let diff:f64 = ((get_current_time_with_ms() - lasttime) as f64) / 1000.0;
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, COLOR_NOTAIL, "{} - ", inconfig.name);
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, COLOR_NOHEAD_NOTAIL, "Incoming: {:.1} regs/sec", (incoming as f64) / diff);
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD_NOTAIL, " | ");
                                        if inconfig.ordering != None {
                                            print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, COLOR_NOHEAD_NOTAIL, "Queue: {} regs", queuer_stat_size);
                                            print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD_NOTAIL, " | ");
                                        }
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, COLOR_NOHEAD_NOTAIL, "Outgoing: {:.1} regs/sec", (outgoing as f64) / diff);
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD_NOTAIL, " | ");
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, COLOR_NOHEAD_NOTAIL, "Dropped: {:.1} regs/sec", (dropped as f64) / diff);
                                        if deleted > 0 {
                                            print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD_NOTAIL, " | ");
                                            print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, COLOR_NOHEAD_NOTAIL, "Deleted: {:.1} regs/sec", (deleted as f64) / diff);
                                        }

                                        // Show stuck clients
                                        let mut stucks = Vec::new();
                                        for element in &stucked {
                                            let (channel, is_stuck) = element.val.clone();
                                            if is_stuck {
                                                stucks.push(format!("{}:{}", element.key, channel));
                                            }
                                        }
                                        if stucks.len() > 0 {
                                            print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, COLOR_NOHEAD_NOTAIL, "  -> Stucked: [ {} ]", stucks.join(", "));
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
                                                Err(e) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Unable to write STATUS at {}: {}", status, e),
                                            }
                                        }

                                        // Show tail
                                        print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD, "");
                                        lasttime = get_current_time_with_ms();
                                        incoming = 0;
                                        outgoing = 0;
                                        dropped = 0;
                                        deleted = 0;
                                    }

                                    // Sleep a sec
                                    thread::sleep(Duration::from_millis(1000));
                                } else {
                                    print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "Some child closed abruptly, closing everything!");
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
                        let mut joined : Vec<u16> = Vec::new();
                        while working>0 {

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
                                        },
                                        // Didn't join, it is alive
                                        Err(_) => working += 1,
                                    }
                                }
                                counter += 1;
                            }

                            // If somebody working, show up
                            if working>0 {
                                print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "There are {} threads left!", working);
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
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, 0, "Couldn't delete PID at {}: {}", pid, why); // why.kind() shows the basic error
                        });
                    }
                }
            }
        }

    } else {

        // Welcome
        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, COLOR_NOHEAD, "{} v{} ({})", PROGRAM_NAME, VERSION, BUILD_DATE);

        // Missing argument, show some help
        print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, COLOR_NOHEAD, "Usage: redismultiplexer <path_to_config.yaml> [debug]");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "Documentation at: https://github.com/juanmitaboada/redismultiplexer");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_CYAN, COLOR_NOHEAD, "Systemd: redismultiplexer <config_prefix> start");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "    <config_prefix>: is the configuration at /etc/redismultiplexer/<config_prefix>.yaml");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "    pid configuration entry will be set as /run/redismultiplexer/<config_prefix>.pid");
        print_debug!(PROGRAM_NAME, stderr(), COLOR_WHITE, COLOR_NOHEAD, "    status configuration entry will be set as /run/redismultiplexer/<config_prefix>.status");
    }
}

/// Do basic checks on configuration
fn verify_config(config: Config) -> Result<Config, String> {

    // To keep sense on source code
    let source = &config;

    // Verify working mode
    if (config.mode!="replicant") && (config.mode!="spreader") {
        return Err(format!("Mode '{}' is unknown, valid modes are: replicant and spreader", config.mode));
    }

    // Verify children
    if config.children<=0 {
        return Err(format!("Children is set to '{}', there is nothing to do without children", config.children));
    }

    // Verify source name
    if source.name.len()==0 {
        return Err(format!("Source '{}' has an empty name [hostname=\"{}\", port={}, channel=\"{}\"]", source.name, source.hostname, source.port, source.channel));
    }

    // Verify source hostname
    if source.hostname.len()==0 {
        return Err(format!("Source '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]", source.name, source.hostname, source.port, source.channel));
    }

    // Verify source channel
    if source.channel.len()==0 {
        return Err(format!("Source '{}' has an empty channel [hostname=\"{}\", port={}, channel=\"{}\"]", source.name, source.hostname, source.port, source.channel));
    }

    // === FILTERS ===

    // Filter
    match &source.filter {
        None => {
            if (source.filter_until != None)
                || (source.filter_limit != None)
                || (source.filter_replace != None) {
                return Err(format!("Source '{}' is using some filtering option but filter is not defined", source.name));
            }
        },
        Some(v) => {
            if v.len()==0 {
                return Err(format!("Source '{}' is using filters, but filter can not empty", source.name));
            }
        },
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
    if (configured>0) && (configured<2) {
        return Err(format!("Source '{}' is using ordering, so you must set all ordering configuration: ordering (not empty), ordering_buffer_time (bigger than 0) and ordering_limit (bigger than 0)", source.name));
    }

    // Verify there are clients
    if config.clients.len() > 0 {

        // Verify all clients
        for client in &config.clients {

            // Verify that name is not empty
            if client.hostname.len()==0 {
                return Err(format!("Client '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]",client.name, client.hostname, client.port, client.channel));
            }

            // Verify that hostname is not empty
            if client.hostname.len()==0 {
                return Err(format!("Client '{}' has an empty hostname [hostname=\"{}\", port={}, channel=\"{}\"]",client.name, client.hostname, client.port, client.channel));
            }

            // Verify that channel is not empty
            if client.channel.len()==0 {
                return Err(format!("Client '{}' has an empty channel [hostname=\"{}\", port={}, channel=\"{}\"]",client.name, client.hostname, client.port, client.channel));
            }

            // Verify that source and target are not the same
            if (source.hostname == client.hostname)
                && (source.port == client.port)
                && (source.channel == client.channel) {
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
            if (configured>0) && (configured<4) {
                return Err(format!("Client '{}' is using limits, so you must set all limits: timelimit, checklimit, softlimit and hardlimit to be bigger than 0", client.name));
            }

            // === FILTERS ===

            // Filter
            match &client.filter {
                None => {
                    if (client.filter_until != None)
                        || (client.filter_limit != None)
                        || (client.filter_replace != None) {
                        return Err(format!("Client '{}' is using some filtering option but filter is not defined", client.name));
                    }
                },
                Some(v) => {
                    if v.len()==0 {
                        return Err(format!("Client '{}' is using filters, but filter can not empty", client.name));
                    }
                },
            }

        }
    } else {
        return Err(format!("No clients found, you need clients to make this to work"));
    }

    return Ok(config);
}

/// Read configuration and parse it from YAML format to Struct
fn get_config(path_to_config:String) -> Result<Config, String> {

    // It this is a systemd call
    let pathconfig = Path::new(&path_to_config);

    // Check if config exists
    if pathconfig.exists() {
        if pathconfig.is_file() {

            #[cfg(feature="debug")]
            print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "CONFIG: Opened {}", path_to_config);

            // Read file
            let data;
            match fs::read_to_string(path_to_config.clone()) {
                Ok(v) => data = v,
                Err(e) => return Err(format!("Couldn't parse configuration at '{}': {}", path_to_config, e)),
            }

            match serde_yaml::from_str(&data[..]) {
                Ok(v) => {
                    // Do basic verifications on configuration
                    match verify_config(v) {
                        Ok(vv) => return Ok(vv),
                        Err(e) => return Err(format!("There is an error in your configuration: {}", e)),
                    };
                },
                Err(e) => return Err(format!("Couldn't parse configuration at '{}': {}", path_to_config, e)),
            }

        } else {
            let error = format!("Couldn't open configuration at '{}': it is not a file!", path_to_config);
            return Err(error);
        }
    } else {
        let error = format!("Configuration at '{}' not found!", path_to_config);
        return Err(error);
    }
}

/// Manage ordered packages in a centralized way
fn queuer(is_ordering_regex: bool, config: Config, keepworking_rx: Receiver<bool>, children_rx: Receiver<(u16, Option<u128>, Option<String>)>, children_tx: Vec<Sender<Vec<String>>>, stat_tx: Sender<Option<usize>>) {

    #[cfg(feature="debug")]
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
    let mut ordered_packages: BinaryHeap<Reverse<(u128, (u64, String))>> = BinaryHeap::new();

    // Prepare the retention data
    while keepworking {

        // Check if some worker requested data
        match children_rx.try_recv() {
            Ok((id, ts, package)) => {

                #[cfg(feature="debug")]
                print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Got request from {}: {:?}", id, package);

                let mut list : Vec<String>;
                if dumpall {
                    list = Vec::new();
                    while let Some(Reverse(package)) = ordered_packages.pop() {
                        list.push(package.1.1);
                    }
                    if let Some(p) = package {
                        list.push(p);
                    }
                } else {
                    list = match_ordering(ts, config.ordering_buffer_time, package, &mut ordered_packages);
                }

                #[cfg(feature="debug")]
                print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Send answer to {}: {}", id, list.len());

                // Answer to the child
                children_tx[id as usize].send(list).unwrap();

                // Send the size of the queue
                stat_tx.send(Some(ordered_packages.len())).unwrap();
            },
            Err(_) => {
                thread::sleep(Duration::from_millis(1));
            },
        }

        // Check if our father wants us to finish
        match keepworking_rx.try_recv() {
            Ok(true) => {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Prepared to die");
                dumpall = true;
            },
            Ok(false) => keepworking = false,
            Err(_) => (),
        }

    }

    // Say we are done
    stat_tx.send(None).unwrap();

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Queue: Ends");

}

/// Manage the full process from a child
fn child(id: u16, ordering_regex: Option<Regex>, ordering_limit: Option<usize>, tx: Sender<Statistics>, rx: Receiver<bool>, qtx: Sender<(u16, Option<u128>, Option<String>)>, qrx: &Receiver<Vec<String>>, config: Config, filter_regex: Option<Regex>) {

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Starts", id);

    // Prepare main variable
    let mut keepworking = true;
    let mut request_finish = false;

    // Prepare the retention data
    while keepworking {

        // Check if our father wants us to finish
        match rx.try_recv() {
            Ok(true) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading message from Parent, got an unexpected message!"),
            Ok(false) => {

                #[cfg(feature="debug")]
                print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "{}: I was told to close!", id);
                request_finish = true;
            },
            Err(_) => (),
        }

        // Connect to source
        let mut source: redis::Connection;
        let mut error = false;
        match redis_connect(id, config.ssl, config.hostname.clone(), config.port, config.password.clone(), false) {
            Ok(link) => {
                source = link;

                // Connect to targets
                let mut clients: Vec<RedisLink> = Vec::new();
                for client in &config.clients {
                    match redis_connect(id, client.ssl, client.hostname.clone(), client.port, client.password.clone(), true) {
                        Ok(link) => {
                            let regex: Option<Regex>;
                            let regex_str: &str;
                            if let Some(r) = client.filter.clone() {
                                regex_str = &r;
                                regex = Some(Regex::new(&regex_str).unwrap());
                            } else {
                                regex = None;
                            }
                            clients.push(RedisLink{
                                config: client.clone(),
                                link: link,
                                sleeping_from: 0,
                                packages: 0,
                                lastcheck: 0,
                                regex: regex,
                            })
                        },
                        Err(e) => {
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while connecting to target Redis Server: {}", e);
                            error = true;
                            break;
                        },
                    }
                }

                // No error until here, keep going
                if !error {

                    // Keep working while allowed
                    let mut incoming: u64= 0;
                    let mut outgoing: u64= 0;
                    let mut dropped: u64= 0;
                    let mut deleted: u64= 0;
                    let mut lasttime = get_current_time();
                    while keepworking {

                        // Check if we should save statistics
                        if (get_current_time() - 1) > lasttime {

                            // Calculate stucked connections
                            let mut stucked: Vec<(String, bool)> = Vec::new();
                            for client in clients.iter_mut() {
                                stucked.push((client.config.name.clone(), client.sleeping_from > 0));
                            }

                            // If we should send statistics
                            let msg = Statistics{
                                _id: id,
                                incoming: incoming,
                                outgoing: outgoing,
                                dropped: dropped,
                                deleted: deleted,
                                stuck: stucked,
                                finished: false,
                            };
                            tx.send(msg).unwrap();

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
                            let item: redis::RedisResult<redis::Value> = source.blpop(config.channel.clone(), 1);
                            match &item {
                                Ok(redis::Value::Nil) => {
                                    // {println!("Nil")},
                                    // Process no data
                                    match process_package(id, &ordering_regex, ordering_limit, &qtx, &qrx, &filter_regex, &config, &mut  clients, None, &mut outgoing, &mut dropped, &mut deleted) {
                                        Ok(_) => (),
                                        Err(e) => {
                                            error = true;
                                            print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{}: process_package() has failed: {}", id, e);
                                        },
                                    }

                                },
                                Ok(redis::Value::Int(_)) => {
                                    // Wrong value
                                    // println!("Int(i64)");
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': not a queue!", config.hostname, config.port);
                                    error=true;
                                },
                                Ok(redis::Value::Data(_)) => {
                                    // Wrong value
                                    // println!("Data(Vec<u8>)");
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': not a queue!", config.hostname, config.port);
                                    error=true;
                                },
                                Ok(redis::Value::Bulk(data)) => {
                                    // This is the expected data
                                    // println!("Bulk(Vec<Value>)");

                                    // Send to all clients
                                    incoming += 1;

                                    // Decode package
                                    if let redis::Value::Data(val) = &data[1] {
                                        match from_utf8(val) {
                                            Ok(raw_bdata) => {
                                                // Got data
                                                match process_package(id, &ordering_regex, ordering_limit, &qtx, &qrx, &filter_regex, &config, &mut  clients, Some(raw_bdata), &mut outgoing, &mut dropped, &mut deleted) {
                                                    Ok(_) => (),
                                                    Err(e) => {
                                                        print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{}: Couldn't process package: {}", id, e);
                                                    },
                                                }
                                            },
                                            Err(e) => {
                                                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Couldn't decode to UTF8: {}", e);
                                                error = true;
                                            }
                                        }
                                    }

                                },
                                Ok(redis::Value::Status(_)) => {
                                    // Wrong value
                                    // println!("Status(String)");
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': not a queue!", config.hostname, config.port);
                                    error=true;
                                },
                                Ok(redis::Value::Okay) => {
                                    // Wrong value
                                    // println!("Okay")
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': not a queue!", config.hostname, config.port);
                                    error = true;
                                },
                                Err(e) => {
                                    // There was an error
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading from Redis Server '{}:{}': {}", config.hostname, config.port, e);
                                    error = true;
                                },
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
                            match process_package(id, &ordering_regex, ordering_limit, &qtx, &qrx, &filter_regex, &config, &mut  clients, None, &mut outgoing, &mut dropped, &mut deleted) {
                                Ok(v) => jobdone = v,
                                Err(e) => {
                                    jobdone = false;
                                    error = true;
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{}: process_package() has failed: {}", id, e);
                                },
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

                    // If we won't keep working
                    if !keepworking {

                        // Calculate stucked connections
                        let mut stucked: Vec<(String, bool)> = Vec::new();
                        for client in clients.iter_mut() {
                            stucked.push((client.config.name.clone(), client.sleeping_from > 0));
                        }

                        // Say we are done
                        let msg = Statistics{
                            _id: id,
                            incoming: incoming,
                            outgoing: outgoing,
                            dropped: dropped,
                            deleted: deleted,
                            stuck: stucked,
                            finished: true,
                        };
                        tx.send(msg).unwrap();

                    }

                } else {

                    thread::sleep(Duration::from_millis(1000));
                }


            },
            Err(e) => {

                // Show error
                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while connecting to source Redis Server '{}:{}': {}", config.hostname, config.port, e);
                thread::sleep(Duration::from_millis(1000));

            },
        }

        // Check if our father wants us to finish
        match rx.try_recv() {
            Ok(true) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while reading message from Parent, got an unexpected message!"),
            Ok(false) => {

                // We are requested to finish
                #[cfg(feature="debug")]
                print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "{}: I was told to close!", id);
                keepworking = false;

            },
            Err(_) => (),
        }

    }

    // Sleep a sec
    thread::sleep(Duration::from_millis(1500));

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Ends", id);
}

fn redis_connect(_id: u16, ssl: Option<bool>, hostname: String, port: u16, password: String, _is_client: bool) -> Result<redis::Connection, String> {

    // If Redis server needs secure connection
    let mut uri_scheme = "redis";
    if let Some(inssl) = ssl {
        if inssl {
            uri_scheme = "rediss";
        }
    }

    // Prepare URL
    let redis_conn_url = format!("{}://:{}@{}:{}", uri_scheme, password, hostname, port);

    #[cfg(feature="debug")]
    {
        let password_debug: String;
        if password.len()>3 {
            password_debug = format!("{}***", &password[..3]);
        } else {
            password_debug = String::from("***");
        }
        let redis_conn_url_debug = format!("{}://:{}@{}:{}", uri_scheme, password_debug, hostname, port);
        if _is_client {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Child {}: Connecting to client: {}", _id, redis_conn_url_debug);
        } else {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Connecting: {}", _id, redis_conn_url_debug);
        }
    }

    match redis::Client::open(redis_conn_url)
        .expect("Invalid connection URL")
        .get_connection() {
        Ok(c) => return Ok(c),
        Err(e) => return Err(format!("Couldn't connect to Redis Server: {}", e)),
    }
}

fn send_to_client(client: &mut RedisLink, data: &str) -> Result<bool, String> {

    // Preparre channels
    let channel  = client.config.channel.clone();

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "RPUSH {} bytes to '{}'!", data.len(), channel);

    let result: redis::RedisResult<i32> = client.link.rpush(&channel, data);
    match result {
        Ok(_) => return Ok(true),
        Err(e) => return Err(format!("couldn't push to channel: {}", e)),
    };
}

fn can_check_queue(timelimit: Option<u64>, checklimit: Option<u64>, packages: u64, lastcheck: u64) -> bool {

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "can_check_queue(): timelimit={:?}  checklimit={:?}   packages={}   lastcheck:{}", timelimit, checklimit, packages, lastcheck);

    // No config
    if (timelimit==None) && (checklimit==None) {
        // No configuration set, using default CHECK SECONDS
        #[cfg(feature="debug")]
        print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "can_check_queue(): No configuration set, using DEFAULT_CHECK_SECONDS={} -> {:?}", DEFAULT_CHECK_SECONDS, (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time());
        return (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time();
    } else {

        // Check by timelimit
        match timelimit {
            None => (),
            Some(ts) => {
                if (lastcheck + ts) < get_current_time() {
                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "can_check_queue(): timelimit=true");
                    return true;
                } else {
                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): timelimit=false");
                }
            }
        }
        // Check by checklimit
        match checklimit {
            None => (),
            Some(_) => {
                if packages == 0 {
                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "can_check_queue(): packages=true");
                    return true;
                } else {
                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): packages=false");
                }
            }
        }
    }

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): default=false");

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
                                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{} - {} :: {} stuck! (Len: {})", id, client.config.name, client.config.channel, len);
                            } else {
                                // Deleteblock in action
                                let mut actual_len:u64 = len as u64;
                                while actual_len >= client.config.hardlimit.unwrap() {

                                    // Trim elements from the queue
                                    let result: redis::RedisResult<redis::Value> = client.link.ltrim(&client.config.channel, client.config.deleteblock.unwrap() as isize, MAX_QUEUE_SIZE);
                                    match result{
                                        Ok(_) => *deleted += client.config.deleteblock.unwrap(),
                                        Err(e) => return Err(format!("error in deleteblock while deleting block with {} elements: {}", client.config.deleteblock.unwrap(), e)),
                                    }

                                    // Read len again
                                    let result: redis::RedisResult<i32> = client.link.llen(&client.config.channel);
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
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_GREEN, 0, "{} - {} :: {} freed! (Len: {})", id, client.config.name, client.config.channel, len);
                        }
                    }
                }

                #[cfg(feature="debug")]
                {
                    if client.sleeping_from==0 {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   true", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    } else {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "{}: can_send(): not stuck yet :: len={}   softlimit={}   hardlimit{}   =>   false", id, len, client.config.softlimit.unwrap(), client.config.hardlimit.unwrap());
                    }
                }

                // If not stuck, can keep sending
                return Ok(client.sleeping_from==0);
            },
            Err(e) => return Err(format!("error requesting the length to the channel: {}", e)),
        };
    } else {

        // Count down packages
        client.packages -= 1;

        #[cfg(feature="debug")]
        {
            if client.sleeping_from==0 {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "{}: can_send(): not stucked :: packages={}   =>   true", id, client.packages);
            } else {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "{}: can_send(): stucked :: packages={}   =>   false", id, client.packages);
            }
        }

        // Return whatever is the status of the queue (we can not check it out)
        return Ok(client.sleeping_from == 0);
    }
}

fn send(id: u16, client: &mut RedisLink, dirty_bdata: &str, deleted: &mut u64) -> Result<bool, String> {


    match match_filter(client.regex.clone(), client.config.filter_until.clone(), client.config.filter_limit, client.config.filter_replace.clone(), dirty_bdata.to_string()) {
        MatchAnswer::Ok(true) => return Err("Programing Error: Unexpected answer from match_filter() at send()".to_string()),
        MatchAnswer::Ok(false) => return Ok(false),
        MatchAnswer::Box(bdata) => {
            // If we can send to this queue
            match can_send(id, client, deleted) {

                // Allowed to send
                Ok(true) => {

                    // Try to send to this client
                    match send_to_client(client, &bdata) {
                        Ok(true) => return Ok(true),
                        Ok(false) => return Ok(false),
                        Err(e) => return Err(format!("error while sending to the client: {}", e)),
                    }
                },

                // Not allowed to send
                Ok(false) => return Ok(false),

                // There was an error
                Err(e) => return Err(format!("error while checking queue: {}", e)),
            }
        },
        MatchAnswer::Err(e) => return Err(format!("couldn't match the package: {}", e)),
    }
}

fn match_filter(regex: Option<Regex>, until: Option<String>, limit: Option<usize>, replace: Option<String>, bdata: String) -> MatchAnswer {

    if let Some(re) = regex {

        // Find by limit
        let slice: &str;
        if let Some(l) = limit {
            if l > 0 {
                slice = &bdata[..cmp::min(l, bdata.len())];
            } else {
                slice = &bdata;
            }
        } else {
            slice = &bdata;
        }

        // Find by until
        let haystack: &str;
        if let Some(u) = until {
            if u.len() > 0 {
                if let Some(idx) = slice.find(&u) {
                    haystack = &slice[..idx];
                } else {
                    haystack = &slice;
                }
            } else {
                haystack = &slice;
            }
        } else {
            haystack = &slice;
        }

        // Check if they match
        if re.is_match(haystack) {
            if let Some(r) = replace {
                let replaced = re.replace(haystack, r);
                let newbdata = format!("{}{}", replaced, &bdata[haystack.len()..]);
                return MatchAnswer::Box(String::from(newbdata));
            } else {
                return MatchAnswer::Box(bdata);
            }
        } else {
            // No match, do not process
            return MatchAnswer::Ok(false);
        }

    } else {
        // No filter found
        return MatchAnswer::Box(bdata);

    }
}

fn match_ordering(ts: Option<u128>, time: Option<u64>, bdata: Option<String>, buffer: &mut BinaryHeap<Reverse<(u128, (u64, String))>>) -> Vec<String> {

    let mut list : Vec<String> = Vec::new();

    // If we got a package
    if let Some(data) = bdata {

        if let Some(v) = ts {

            buffer.push(Reverse((v, (get_current_time(), data.to_string()))));

        } else {
            // No filter available, just send it
            list.push(data.to_string());
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
        if package.1.0 < control {
            let Reverse(p) = buffer.pop().unwrap();
            list.push(p.1.1);
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

fn process_package(id: u16, ordering_regex: &Option<Regex>, ordering_limit: Option<usize>, qtx: &Sender<(u16, Option<u128>, Option<String>)>, qrx: &Receiver<Vec<String>>, filter_regex: &Option<Regex>, config: &Config, clients: &mut Vec<RedisLink>, package: Option<&str>, outgoing: &mut u64, dropped: &mut u64, deleted: &mut u64) -> Result<bool, String> {

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: Start process_package(): data={:?}", id, package);

    // Check if we got a package
    if let Some(data) = package {

        let ts: Option<u128>;

        // Process regex
        if let Some(re) = ordering_regex {

            // Find by limit
            let haystack: &str;
            if let Some(l) = ordering_limit {
                if l > 0 {
                    haystack = &data[..cmp::min(l, data.len())];
                } else {
                    haystack = &data;
                }
            } else {
                haystack = &data;
            }

            // Check if they match
            match re.captures(haystack) {
                Some(x) => {
                    let parsed_ts = x.name("ts").map_or("", |m| m.as_str());
                    match parsed_ts.parse::<u128>() {
                        Ok(n) => ts=Some(n),
                        Err(_) => {
                            // No TS information, jut send it
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, 0, "Found a package with ordering information but 'ts' couldn't be parsed to u128");
                            ts = None;
                        },
                    }
                },
                None => {
                    // No TS information, jut send it
                    print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, 0, "Found a package without ordering information");
                    ts = None;
                },
            }

        } else {
            // No filter available, just send it
            ts = None;
        }

        // Send it to queuer
        qtx.send((id, ts, Some(data.to_string()))).unwrap();
    } else {
        // Just say we didn't get anything
        qtx.send((id, None, None)).unwrap();
    }

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: Sent request to Queuer process_package()", id);

    // Check if there is some work to be done
    let list:Vec<String> = qrx.recv().unwrap();

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: Got answer from Queuer process_package(): {}", id, list.len());

    // Check if we got packages to send
    let jobdone: bool;
    if list.len() > 0 {

        for package in list {

            // Ready to send data
            let total_clients = clients.len();
            let mut errors = 0;

            match match_filter(filter_regex.clone(), config.filter_until.clone(), config.filter_limit, config.filter_replace.clone(), package.to_string()) {
                MatchAnswer::Ok(true) => {

                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: End unexpected process_package()", id);

                    return Err("Programing Error: Unexpected answer from match_filter() at process_package()".to_string());
                },
                MatchAnswer::Ok(false) => errors = total_clients,
                MatchAnswer::Box(bdata) => {

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
                                },

                                // There was an error
                                Err(e) => {
                                    // There was an error
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while sending to '{}:{}@{}': {}", config.hostname, config.port, config.channel, e);
                                    errors += 1;
                                },
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
                                },

                                // There was an error
                                Err(e) => {
                                    // There was an error
                                    print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while sending to '{}:{}@{}': {}", config.hostname, config.port, config.channel, e);
                                    errors += 1;
                                },
                            }

                            // Rotate
                            if total_clients > 1 {
                                for i in 0..(total_clients-1) {
                                    clients.swap(i, i+1);
                                }
                            }

                            // Leave if we are done
                            if done {
                                break;
                            }
                        }
                    }
                },
                MatchAnswer::Err(e) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{}: Couldn't match the package: {}", id, e),
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
                Ok(_) => (),  // We do not care if it can send or not (just wanted to refresh client information)
                Err(e) => {

                    #[cfg(feature="debug")]
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: End unexpected process_package()", id);

                    return Err(format!("couldn't check queue for {}: {}", client.config.channel, e));
                },
            }
        }
        jobdone = false;
    }

    #[cfg(feature="debug")]
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{}: End process_package() - JOBDONE: {:?}", id, jobdone);

    // Send if we did or didn't do the job
    return Ok(jobdone);
}
