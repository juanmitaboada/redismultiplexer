//! Ordering buffer.

mod common;

use common::*;
use std::thread;
use std::time::Duration;

const ORDERING: &str =
    "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: {T}\nordering_limit: 200\n";

fn pkg(ts: u64) -> String {
    format!("{{\"ts\": {}, \"v\": \"x\"}}", ts)
}

/// 0..n in a fixed shuffled order (multiplicative step, n coprime with 7).
fn shuffled(n: u64) -> Vec<u64> {
    (0..n).map(|i| (i * 7 + 3) % n).collect()
}

#[test]
fn ordering_sorts_within_buffer() {
    let mut ctx = Ctx::new("ordering_sort");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let mut input: Vec<String> = shuffled(50).into_iter().map(pkg).collect();
    // A package without a timestamp is delivered at once, ahead of the buffer.
    input.insert(25, "no timestamp here".to_string());
    ctx.push(&src, &input);
    let extra = ORDERING.replace("{T}", "1");
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        &extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 51)]);
    p.stop(&ctx.pid_file());
    let got = ctx.list(&t0);
    assert_eq!(got[0], "no timestamp here");
    let want: Vec<String> = (0..50).map(pkg).collect();
    assert_eq!(got[1..], want[..]);
    assert!(p.stderr().contains("without ordering information"));
}

#[test]
fn ordering_buffer_flushed_on_sigint() {
    let mut ctx = Ctx::new("ordering_flush");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let input: Vec<String> = shuffled(10).into_iter().map(pkg).collect();
    ctx.push(&src, &input);
    let extra = ORDERING.replace("{T}", "30");
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        &extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 0)]);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(ctx.llen(&t0), 0, "packages left the 30 s buffer early");
    p.stop(&ctx.pid_file());
    let want: Vec<String> = (0..10).map(pkg).collect();
    assert_eq!(ctx.list(&t0), want);
}

#[test]
fn ordering_stop_under_load_loses_nothing() {
    // M2R-1: one child, a busy source and a full 1 s buffer when SIGINT arrives. Every
    // package must end up either delivered or still in the source, exactly once. A queuer
    // that answers the child's last request without flushing loses the buffer; that race
    // is not hit on every stop, so the deterministic check is the queuer unit test.
    // M2R-7: the source is fed while the program runs, so no build drains it first.
    stop_under_load("ordering_stop_load", 1, true, Duration::from_millis(300));
}
