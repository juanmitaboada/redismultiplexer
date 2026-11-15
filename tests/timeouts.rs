//! The `connect_timeout` and `command_timeout` keys (M3R-19 = R19a, S4-1, S4-2): their
//! defaults, 0 as 1.0.8's wait, and the source's own timeouts. The keys set the socket of
//! one connection, the same code for a worker's targets, the sender's and the source, so
//! these tests run one cell each; the failure tests run the matrix with short values.

mod common;

use common::*;
use std::thread;
use std::time::{Duration, Instant};

/// Seconds from the start until `line` is first in the program's stderr
fn first_line_after(p: &Proc, line: &str, limit: Duration) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if p.stderr().contains(line) {
            return Some(start.elapsed());
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

/// One target that does not answer, one package to send to it: the time until the
/// program gives up on it is the timeout in force
fn give_up_time(
    test: &str,
    port: u16,
    keys: &str,
    line: &str,
    limit: Duration,
) -> Option<Duration> {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let client = ctx.client_yaml_at("mute", "rmtest:mute", port) + keys;
    let cfg = ctx.config(&src, 1, "replicant", "", &[client]);
    let p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    let t = first_line_after(&p, line, limit);
    // With no limit the thread may still wait for the target: no clean stop to assert
    p.signal("INT");
    t
}

#[test]
fn connect_timeout_default_is_two_seconds() {
    let hole = blackhole();
    let t = give_up_time(
        "ct_default",
        hole.port,
        "",
        "Error while connecting",
        Duration::from_secs(10),
    )
    .expect("no connect error within 10 s");
    assert!(
        t >= Duration::from_millis(1800) && t < Duration::from_secs(4),
        "gave up after {:?}",
        t
    );
}

#[test]
fn connect_timeout_key() {
    let hole = blackhole();
    let t = give_up_time(
        "ct_key",
        hole.port,
        "    connect_timeout: 0.3\n",
        "Error while connecting",
        Duration::from_secs(10),
    )
    .expect("no connect error within 10 s");
    assert!(t < Duration::from_millis(1500), "gave up after {:?}", t);
}

#[test]
fn command_timeout_default_is_five_seconds() {
    let mute = SilentTarget::start();
    let t = give_up_time(
        "mt_default",
        mute.port,
        "",
        // The first command that waits is CLIENT SETNAME, at the connect
        "Error while connecting",
        Duration::from_secs(15),
    )
    .expect("no send error within 15 s");
    assert!(
        t >= Duration::from_millis(4800) && t < Duration::from_secs(8),
        "gave up after {:?}",
        t
    );
}

#[test]
fn command_timeout_key() {
    let mute = SilentTarget::start();
    let t = give_up_time(
        "mt_key",
        mute.port,
        "    command_timeout: 0.4\n",
        "Error while connecting",
        Duration::from_secs(10),
    )
    .expect("no send error within 10 s");
    assert!(t < Duration::from_millis(1500), "gave up after {:?}", t);
}

#[test]
fn command_timeout_zero_waits_as_1_0_8() {
    // 0: no limit, the thread waits for the target as 1.0.8 did
    let mute = SilentTarget::start();
    let t = give_up_time(
        "mt_zero",
        mute.port,
        "    command_timeout: 0\n",
        "Error while",
        Duration::from_secs(7),
    );
    assert_eq!(t, None, "gave up although the key asks to wait");
}

/// A silent source with the default timeouts: a worker waits in its BLPOP at most 1 s
/// (BLPOP's own wait) plus the 5 s command timeout, then sees the stop. The bound of a stop
/// with a source that does not answer (C4a-4: a late reply cannot be read after a timeout,
/// so the timeout stays the stop's bound)
#[test]
fn silent_source_default_bounds_the_stop() {
    let mute = SilentTarget::start();
    let mut ctx = Ctx::new("silent_source_default");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(
        "rmtest:mute_src",
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let yaml = std::fs::read_to_string(&cfg).unwrap();
    let root = format!(
        "hostname: {}\nport: {}\npassword: {}\n",
        q(&ctx.ep.host),
        ctx.ep.port,
        q(&ctx.ep.password)
    );
    let mute_root = format!("hostname: '127.0.0.1'\nport: {}\npassword: ''\n", mute.port);
    std::fs::write(&cfg, yaml.replacen(&root, &mute_root, 1)).unwrap();
    let mut p = ctx.spawn(&cfg);
    thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    p.stop(&ctx.pid_file());
    let took = start.elapsed();
    assert!(took < Duration::from_secs(8), "stop took {:?}", took);
}

#[test]
fn silent_source_does_not_hold_the_stop() {
    // S4-2: a source that accepts and never answers. 1.0.8 waited in BLPOP for ever and
    // never saw the stop; with the source's command timeout the worker gives up, reconnects
    // and sees the stop flag.
    let mute = SilentTarget::start();
    let mut ctx = Ctx::new("silent_source");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(
        "rmtest:mute_src",
        2,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let yaml = std::fs::read_to_string(&cfg).unwrap();
    let root = format!(
        "hostname: {}\nport: {}\npassword: {}\n",
        q(&ctx.ep.host),
        ctx.ep.port,
        q(&ctx.ep.password)
    );
    let mute_root = format!(
        "hostname: '127.0.0.1'\nport: {}\npassword: ''\ncommand_timeout: 0.5\n",
        mute.port
    );
    assert!(yaml.starts_with("name: 'source'\nssl: "), "{}", yaml);
    std::fs::write(&cfg, yaml.replacen(&root, &mute_root, 1)).unwrap();
    let mut p = ctx.spawn(&cfg);
    // Each worker is in its first BLPOP against the silent source
    thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    p.stop(&ctx.pid_file());
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "stop took {:?}",
        start.elapsed()
    );
}

/// The sender's connections take the timeout keys too (M3R-25): with ordering the sender
/// waits at its start for a target that never answers up to that target's connect plus
/// command timeout, so a healthy target's first ordered package comes after the short
/// bound the keys give (0.3 + 0.4 s) plus the 1 s buffer, not after the 7 s of the defaults
fn sender_honours_the_keys(test: &str, port: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let keys = "    connect_timeout: 0.3\n    command_timeout: 0.4\n";
    let clients = [
        ctx.client_yaml_at("mute", "rmtest:mute", port) + keys,
        ctx.client_yaml("t0", &t0, ""),
    ];
    let cfg = ctx.config(&src, 1, "replicant", ORDERING_1S, &clients);
    let start = Instant::now();
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_for("the package at the healthy target", |c| c.llen(&t0) == 1);
    let took = start.elapsed();
    p.stop(&ctx.pid_file());
    assert!(
        took < Duration::from_secs(4),
        "first ordered package after {:?}",
        took
    );
}

#[test]
fn sender_connect_timeout_key() {
    let hole = blackhole();
    sender_honours_the_keys("sender_ct_key", hole.port);
}

#[test]
fn sender_connect_timeout_above_the_default() {
    // M3R-39: a key above the 2 s default gives a lower bound that does not depend on
    // load. The worker and the sender each try the target that never connects (one error
    // line each); with the key neither line comes before 4 s, while a sender on the 2 s
    // default prints its own at 2 s
    let hole = blackhole();
    let mut ctx = Ctx::new("sender_ct_above");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let clients = [
        ctx.client_yaml_at("hole", "rmtest:hole", hole.port) + "    connect_timeout: 4\n",
        ctx.client_yaml("t0", &t0, ""),
    ];
    let cfg = ctx.config(&src, 1, "replicant", ORDERING_1S, &clients);
    let start = Instant::now();
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    let lines = |p: &Proc| p.stderr().matches("Error while connecting").count();
    let mut first = None;
    while lines(&p) < 2 && start.elapsed() < Duration::from_secs(10) {
        if first.is_none() && lines(&p) > 0 {
            first = Some(start.elapsed());
        }
        thread::sleep(Duration::from_millis(20));
    }
    let first = first.unwrap_or_else(|| start.elapsed());
    ctx.wait_for("the package at the healthy target", |c| c.llen(&t0) == 1);
    p.stop(&ctx.pid_file());
    assert!(lines(&p) >= 2, "{}", p.stderr());
    assert!(
        first >= Duration::from_millis(3500),
        "a connect error after {:?}",
        first
    );
}

#[test]
fn sender_command_timeout_key() {
    let mute = SilentTarget::start();
    sender_honours_the_keys("sender_mt_key", mute.port);
}

/// The command timeout bounds a write too (M3R-26): a target that answers its handshake and
/// the idle PING and reads nothing of an RPUSH; a package larger than the socket buffers blocks the RPUSH in the
/// write, which gives up after the command timeout; the healthy target then gets it
fn write_timeout(test: &str, ordering: bool) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let deaf = DeafTarget::start();
    let clients = [
        ctx.client_yaml_at("deaf", "rmtest:deaf", deaf.port) + "    command_timeout: 0.5\n",
        ctx.client_yaml("t0", &t0, ""),
    ];
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(&src, 1, "replicant", extra, &clients);
    let mut p = ctx.spawn(&cfg);
    // 64 MiB: far above what loopback buffers hold
    let big = format!("{}{}", seq_pkg(1), "x".repeat(64 << 20));
    ctx.push(&src, &[big]);
    ctx.wait_for("the big package at the healthy target", |c| {
        c.llen(&t0) == 1
    });
    p.stop(&ctx.pid_file());
    assert!(p.stderr().contains("Error while sending"), "{}", p.stderr());
}

#[test]
fn write_timeout_bounds_a_target_that_stops_reading() {
    write_timeout("write_timeout_off", false);
}

#[test]
fn write_timeout_bounds_a_target_that_stops_reading_sender() {
    write_timeout("write_timeout_on", true);
}

/// The source read waits BLPOP's own second beyond the command timeout (M3R-26): with
/// `command_timeout: 0.5` at the root an idle source is not a timeout (each BLPOP answers
/// nil after 1 s), so no read error and no reconnect while idle
#[test]
fn source_read_waits_for_blpop() {
    let mut ctx = Ctx::new("source_blpop_wait");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let extra = "command_timeout: 0.5\n";
    let cfg = ctx.config(
        &src,
        2,
        "replicant",
        extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    thread::sleep(Duration::from_millis(3500));
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    p.stop(&ctx.pid_file());
    assert!(
        !p.stderr().contains("Error while reading"),
        "{}",
        p.stderr()
    );
}
