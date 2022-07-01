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
use std::cmp;
use std::str::from_utf8;
use thread_tryjoin::TryJoinHandle;
use std::time::Duration;
use std::io::{stdout, stderr, Write};
use serde::{Serialize, Deserialize};
use std::sync::mpsc::{Sender, Receiver};
use std::sync::mpsc;
use regex::Regex;
use sorted_list::SortedList;

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
    filter: Option<String>,
    filter_until: Option<String>,
    filter_limit: Option<usize>,
    filter_replace: Option<String>,
    ordering: Option<String>,
    ordering_buffer_time: Option<u64>,
    ordering_limit: Option<usize>,
    ordering_prets: Option<String>,
    ordering_posts: Option<String>,
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
            filter: self.filter.clone(),
            filter_until: self.filter_until.clone(),
            filter_limit: self.filter_limit,
            filter_replace: self.filter_replace.clone(),
            ordering: self.ordering.clone(),
            ordering_buffer_time: self.ordering_buffer_time,
            ordering_limit: self.ordering_limit,
            ordering_prets: self.ordering_prets.clone(),
            ordering_posts: self.ordering_posts.clone(),
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
}

/// Main module will manage the basics from this program
fn main() {

    // Welcome
    print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "{} v{} ({} - {})", PROGRAM_NAME, VERSION, BUILD_VERSION, BUILD_DATE);

    // Get args
    let args: Vec<String> = std::env::args().collect();

    // Check if we got instructed with the path to configuration
    if args.len() > 1 {

        // Set debug
        let debug = DEBUG || (args.len() > 2) && (args[2] == "debug");
        if debug {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "Debug is enabled!");
        }

        // Read config
        let mut config : Option<Config> = None;
        match get_config(&args[1], debug) {
            Ok(v) => config = Some(v),
            Err(e) => print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while processing configuration: {}", e),
        }

        // Check if we can keep working
        if let Some(inconfig) = config {

            // Set handler
            let (keepworking_tx, keepworking_rx): (Sender<i8>, Receiver<i8>) = mpsc::channel();
            ctrlc::set_handler(move || {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "User requested to exit!");
                keepworking_tx.send(1).unwrap();
            }).expect("Error setting Ctrl-C handler");

            // Let communicate with children to end
            let (children_tx, children_rx): (Sender<Statistics>, Receiver<Statistics>) = mpsc::channel();

            // Spawn a number of threads and collect their join handles
            let mut id = 1;
            let mut handles: Vec<thread::JoinHandle<_>> = Vec::new();
            let mut channels: Vec<Sender<i8>> = Vec::new();
            for _ in 0..inconfig.children {

                // Get channel for the child
                let (tx, rx): (Sender<i8>, Receiver<i8>) = mpsc::channel();
                // Remember the channel so we can talk later with the child
                channels.push(tx);

                // Clone config and execute thread
                let tx = children_tx.clone();
                let child_config = inconfig.clone();
                let handle = thread::spawn(move || {
                    child(id, tx, rx, child_config, debug);
                });
                handles.push(handle);

                id += 1;

                // Do not rush
                thread::sleep(Duration::from_millis(1));
            }

            // Show configuration
            print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "> {}:{} @ {} [{}, {} children]", inconfig.hostname, inconfig.port, inconfig.channel, inconfig.mode, inconfig.children);
            for client in &inconfig.clients {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "  - {}:{} @ {}  [timelimit={}, checklimit={}, softlimit={}, hardlimit={}]", client.hostname, client.port, client.channel, option2string!(client.timelimit), option2string!(client.checklimit), option2string!(client.softlimit), option2string!(client.hardlimit));
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
            let mut working = inconfig.children;
            while working>0 {

                // Check how many chidren we should have
                working = inconfig.children;

                // Check how many of them have stopped
                for h in &handles {
                    match h.try_timed_join(Duration::from_millis(1)) {
                        Ok(_) => working -= 1,
                        Err(_) => (),
                    }
                }

                // If somebody working, show up
                if working<inconfig.children {

                    // Some child already died, show message and leave
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "Children {}/{}, some child closed, closing everything!", working, inconfig.children);
                    break;
                } else {

                    // Check if there is some message for us (from CTRL+C)
                    let result_incoming = keepworking_rx.try_recv();
                    match result_incoming {
                        Ok(v) => {
                            if v==1 {
                                working = 0;
                            }
                        },
                        Err(_) => (),
                    }

                    // If nothind changed with children
                    if working==inconfig.children {

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

                        // Check if we should show statistics
                        if (get_current_time_with_ms() - (STATISTICS_SECONDS*1000))  > lasttime {
                            // Show statistics
                            let diff:f64 = ((get_current_time_with_ms() - lasttime) as f64) / 1000.0;
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, COLOR_NOTAIL, "{} - ", inconfig.name);
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, COLOR_NOHEAD_NOTAIL, "Incoming: {:.1} regs/sec", (incoming as f64) / diff);
                            print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, COLOR_NOHEAD_NOTAIL, " | ");
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

                    }
                }

            }

            // Tell all children to close
            for channel in channels {
                channel.send(1).unwrap();
            }
            thread::sleep(Duration::from_millis(1));

            // Wait for children to close
            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Closing...");
            thread::sleep(Duration::from_millis(1000));

            // Wait for all children to close
            let mut working = 1;
            while working>0 {
                working = 0;
                for h in &handles {
                    match h.try_timed_join(Duration::from_millis(1)) {
                        Ok(_) => (),
                        Err(_) => working += 1,
                    }
                }

                // If somebody working, show up
                if working>0 {
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_YELLOW, 0, "There are {} threads left!", working);
                    thread::sleep(Duration::from_millis(1000));
                }
            }

            print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "Program finished!");

        }

    } else {
        // Missing argument
        print_debug!(PROGRAM_NAME, stderr(), COLOR_YELLOW, 0, "Usage: {} <path_to_config.yaml>", args[0]);
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

    // Ordering Pre TS
    match &source.ordering_prets {
        None => (),
        Some(_) => configured += 1,
    }

    // Ordering Post TS
    match &source.ordering_posts {
        None => (),
        Some(_) => configured += 1,
    }

    // Show error if any
    if (configured>0) && (configured<4) {
        return Err(format!("Source '{}' is using ordering, so you must set all ordering configuration: ordering (not empty), ordering_buffer_time (bigger than 0), ordering_limit (bigger than 0), ordering_prets and ordering_posts", source.name));
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
fn get_config(path_to_config:&String, debug:bool) -> Result<Config, String> {

    let path = Path::new(path_to_config);

    // Check if config exists
    if path.exists() {
        if path.is_file() {

            if debug {
                print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "CONFIG: Opened {}", path_to_config);
            }

            // Read file
            let data;
            match fs::read_to_string(path_to_config) {
                Ok(v) => data = v,
                Err(e) => return Err(format!("Couldn't parse configuration at '{}': {}", path_to_config, e)),
            }

            match serde_yaml::from_str(&data[..]) {
                Ok(v) => {
                    // Do basic verifications on configuration
                    match verify_config(v) {
                        Ok(v) => return Ok(v),
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


/// Manage the full process from a child
fn child(id: u16, tx: Sender<Statistics>, rx: Receiver<i8>, config: Config, debug: bool) {

    if debug {
        print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Starts", id);
    }

    // Render main filter regex
    let filter_regex: Option<Regex>;
    let regex_str: &str;
    if let Some(r) = config.filter.clone() {
        regex_str = &r;
        filter_regex = Some(Regex::new(&regex_str).unwrap());
    } else {
        filter_regex = None;
    }

    // Render main ordering regex
    let ordering_regex: Option<Regex>;
    let regex_str: &str;
    if let Some(r) = config.ordering.clone() {
        regex_str = &r;
        ordering_regex = Some(Regex::new(&regex_str).unwrap());
    } else {
        ordering_regex = None;
    }


    // Prepare sorted list
    let mut ordered_packages : SortedList<u128, (u64, String)> = SortedList::new();

    // Prepare the retention data
    let mut keepworking = true;
    while keepworking {

        // Connect to source
        let mut source: redis::Connection;
        match redis_connect(id, config.ssl, config.hostname.clone(), config.port, config.password.clone(), false, debug) {
            Ok(link) => {
                let mut error = false;
                source = link;

                // Connect to targets
                let mut clients: Vec<RedisLink> = Vec::new();
                for client in &config.clients {
                    match redis_connect(id, client.ssl, client.hostname.clone(), client.port, client.password.clone(), true, debug) {
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
                            print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while connecting to Redis Server: {}", e);
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
                            };
                            tx.send(msg).unwrap();

                            // Reset status
                            incoming = 0;
                            outgoing = 0;
                            dropped = 0;
                            deleted = 0;
                            lasttime = get_current_time();
                        }

                        // Get a new package
                        let item: redis::RedisResult<redis::Value> = source.blpop(config.channel.clone(), 1);
                        match &item {
                            Ok(redis::Value::Nil) => {
                                // Refresh clients status
                                // {println!("Nil")},
                                for client in clients.iter_mut() {
                                    match can_send(id, client, &mut deleted, debug) {
                                        Ok(_) => (),
                                        Err(e) => {
                                            print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "{}: Couldn't check queue for {}: {}", id, client.config.channel, e);
                                            error = true;
                                        },
                                    }
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

                                            match match_ordering(ordering_regex.clone(), config.ordering_buffer_time, config.ordering_limit, config.ordering_prets.clone(), config.ordering_posts.clone(), raw_bdata.to_string(), &mut ordered_packages) {
                                                Ok(list) => {

                                                    for package in list {

                                                        // Ready to send data
                                                        let total_clients = clients.len();
                                                        let mut errors = 0;

                                                        match match_filter(filter_regex.clone(), config.filter_until.clone(), config.filter_limit, config.filter_replace.clone(), package.to_string()) {
                                                            MatchAnswer::Ok(_) => errors = total_clients,
                                                            MatchAnswer::Box(bdata) => {

                                                                if config.mode == "replicant" {

                                                                    // Send data to all clients
                                                                    for client in clients.iter_mut() {

                                                                        // If we can send to this queu
                                                                        match send(id, client, &bdata, &mut deleted, debug) {

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
                                                                        match send(id, client, &bdata, &mut deleted, debug) {

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
                                                            dropped += 1;
                                                        } else {
                                                            // The package was sent at least to 1 node
                                                            outgoing += 1;
                                                        }
                                                    }
                                                },
                                                Err(String) => (),
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
                            Ok(v) => {
                                if v==1 {
                                    keepworking = false;
                                }
                            },
                            Err(_) => (),
                        }

                        // Error found while processing
                        if error && keepworking {
                            thread::sleep(Duration::from_millis(1000));
                            break;
                        }

                    }

                } else {

                    thread::sleep(Duration::from_millis(1000));
                }


            },
            Err(e) => {
                print_debug!(PROGRAM_NAME, stderr(), COLOR_RED, 0, "Error while connecting to Redis Server '{}:{}': {}", config.hostname, config.port, e);
                thread::sleep(Duration::from_millis(1000));
            },
        }


        // Check if our father wants us to finish
        match rx.try_recv() {
            Ok(v) => {
                if v==1 {
                    keepworking = false;
                }
            },
            Err(_) => (),
        }

    }

    if debug {
        print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Ends", id);
    }
}

fn redis_connect(id: u16, ssl: Option<bool>, hostname: String, port: u16, password: String, is_client: bool, debug: bool) -> Result<redis::Connection, String> {

    // If Redis server needs secure connection
    let mut uri_scheme = "redis";
    if let Some(inssl) = ssl {
        if inssl {
            uri_scheme = "rediss";
        }
    }

    // Prepare URL
    let redis_conn_url = format!("{}://:{}@{}:{}", uri_scheme, password, hostname, port);

    if debug {
        let password_debug: String;
        if password.len()>3 {
            password_debug = format!("{}***", &password[..3]);
        } else {
            password_debug = String::from("***");
        }
        let redis_conn_url_debug = format!("{}://:{}@{}:{}", uri_scheme, password_debug, hostname, port);
        if is_client {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "Child {}: Connecting to client: {}", id, redis_conn_url_debug);
        } else {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_BLUE, 0, "Child {}: Connecting: {}", id, redis_conn_url_debug);
        }
    }

    match redis::Client::open(redis_conn_url)
        .expect("Invalid connection URL")
        .get_connection() {
        Ok(c) => return Ok(c),
        Err(e) => return Err(format!("Couldn't connect to Redis Server: {}", e)),
    }
}

fn send_to_client(client: &mut RedisLink, data: &str, debug: bool) -> Result<bool, String> {
    let channel  = client.config.channel.clone();
    if debug {
        print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "RPUSH {} bytes to '{}'!", data.len(), channel);
    }
    let result: redis::RedisResult<i32> = client.link.rpush(&channel, data);
    match result {
        Ok(_) => return Ok(true),
        Err(e) => return Err(format!("couldn't push to channel: {}", e)),
    };
}

fn can_check_queue(timelimit: Option<u64>, checklimit: Option<u64>, packages: u64, lastcheck: u64, debug:bool) -> bool {

    // Debugger
    if debug {
        print_debug!(PROGRAM_NAME, stdout(), COLOR_CYAN, 0, "can_check_queue(): timelimit={:?}  checklimit={:?}   packages={}   lastcheck:{}", timelimit, checklimit, packages, lastcheck);
    }

    // No config
    if (timelimit==None) && (checklimit==None) {
        // No configuration set, using default CHECK SECONDS
        if debug {
            print_debug!(PROGRAM_NAME, stdout(), COLOR_WHITE, 0, "can_check_queue(): No configuration set, using DEFAULT_CHECK_SECONDS={} -> {:?}", DEFAULT_CHECK_SECONDS, (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time());
        }
        return (lastcheck + DEFAULT_CHECK_SECONDS) < get_current_time();
    } else {

        // Check by timelimit
        match timelimit {
            None => (),
            Some(ts) => {
                if (lastcheck + ts) < get_current_time() {
                    if debug {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "can_check_queue(): timelimit=true");
                    }
                    return true;
                } else if debug {
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): timelimit=false");
                }
            }
        }
        // Check by checklimit
        match checklimit {
            None => (),
            Some(_) => {
                if packages == 0 {
                    if debug {
                        print_debug!(PROGRAM_NAME, stdout(), COLOR_GREEN, 0, "can_check_queue(): packages=true");
                    }
                    return true;
                } else if debug {
                    print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): packages=false");
                }
            }
        }
    }

    if debug {
        print_debug!(PROGRAM_NAME, stdout(), COLOR_RED, 0, "can_check_queue(): default=false");
    }
    return false;
}

fn can_send(id: u16, client: &mut RedisLink, deleted: &mut u64, debug: bool) -> Result<bool, String> {

    // Check if we can check queue
    if can_check_queue(
        client.config.timelimit,
        client.config.checklimit,
        client.packages,
        client.lastcheck,
        debug
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

                if debug {
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

        if debug {
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

fn send(id: u16, client: &mut RedisLink, dirty_bdata: &str, deleted: &mut u64, debug: bool) -> Result<bool, String> {


    match match_filter(client.regex.clone(), client.config.filter_until.clone(), client.config.filter_limit, client.config.filter_replace.clone(), dirty_bdata.to_string()) {
        MatchAnswer::Ok(_) => return Ok(false),
        MatchAnswer::Box(bdata) => {
            // If we can send to this queue
            match can_send(id, client, deleted, debug) {

                // Allowed to send
                Ok(true) => {

                    // Try to send to this client
                    match send_to_client(client, &bdata, debug) {
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

fn match_ordering(regex: Option<Regex>, time: Option<u64>, limit: Option<usize>, pretts: Option<String>, postts: Option<String>, bdata: String, buffer: &mut SortedList<u128, (u64, String)>) -> Result<Vec<String>, String> {

    let mut list : Vec<String> = Vec::new();

    // Attach all packages from the buffer that should be sent already


    // Process regex
    if let Some(re) = regex {

        // Find by limit
        let haystack: &str;
        if let Some(l) = limit {
            if l > 0 {
                haystack = &bdata[..cmp::min(l, bdata.len())];
            } else {
                haystack = &bdata;
            }
        } else {
            haystack = &bdata;
        }

        // Check if they match
        if re.is_match(haystack) {

            if let Some(r) = pretts {
                let replaced = re.replace(haystack, r);
                let newbdata = format!("{}{}", replaced, &bdata[haystack.len()..]);
            }

            // Post TS
            if let Some(r) = postts {
                let replaced = re.replace(haystack, r);
                let newbdata = format!("{}{}", replaced, &bdata[haystack.len()..]);
            }

            // Parse to u128
            let ts: u128 = 0;

            // Insert package into the buffer
            buffer.insert(ts, (get_current_time(), "b".to_string()));

        } else {

            // No TS information, jut send it
            list.push(bdata);

        }

    } else {
        // No filter available, just send it
        list.push(bdata);
    }

    // Return the list back
    return Ok(list);

}
