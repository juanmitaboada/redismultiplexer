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
