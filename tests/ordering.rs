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

/// A full 30 s buffer is delivered in ts order when the program is stopped, by any of the
/// stop signals (M3-l = la: before, SIGTERM and SIGHUP killed it and lost the buffer).
fn buffer_flushed(test: &str, sig: &str) {
    let mut ctx = Ctx::new(test);
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
    p.stop_with(&ctx.pid_file(), sig);
    let want: Vec<String> = (0..10).map(pkg).collect();
    assert_eq!(ctx.list(&t0), want);
}

#[test]
fn ordering_buffer_flushed_on_sigint() {
    buffer_flushed("ordering_flush", "INT");
}

#[test]
fn ordering_buffer_flushed_on_sigterm() {
    buffer_flushed("ordering_flush_term", "TERM");
}

#[test]
fn ordering_buffer_flushed_on_sighup() {
    buffer_flushed("ordering_flush_hup", "HUP");
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

/// Packages with increasing ts, pushed in that order while the program runs, must reach
/// the target in exactly that order with any number of children (M3-k = ka, M3F-6).
/// 1.0.8 and main hand each second's packages to whichever child asks next, and the
/// children deliver those batches at the same time. The target is a SlowTarget taking
/// 2 ms per RPUSH and the source is fed at 1000 packages per second, so delivering one
/// second's batch takes at least 2 s: the next batch always starts in another child while
/// the previous one is still being delivered, on any machine, and their packages mix. One
/// sender delivers batch after batch, so the order holds.
fn strict_order(test: &str, children: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let target = SlowTarget::start(Duration::from_millis(2));
    let n: u64 = 3000;
    let cfg = ctx.config(
        &src,
        children,
        "replicant",
        ORDERING_1S,
        &[ctx.client_yaml_at("t0", "rmtest:slow", target.port)],
    );
    let mut p = ctx.spawn(&cfg);
    let producer = {
        let (url, src) = (ctx.ep.url.clone(), src.clone());
        thread::spawn(move || {
            let mut con = redis::Client::open(url.as_str())
                .unwrap()
                .get_connection()
                .unwrap();
            for start in (0..n).step_by(50) {
                let batch: Vec<String> = (start..start + 50).map(seq_pkg).collect();
                let _: usize = redis::Commands::rpush(&mut con, &src, batch).unwrap();
                thread::sleep(Duration::from_millis(50));
            }
        })
    };
    producer.join().unwrap();
    let all = n as usize;
    ctx.wait_for("every package at the slow target", |c| {
        c.llen(&src) == 0 && target.count() == all
    });
    p.stop(&ctx.pid_file());
    let want: Vec<String> = (0..n).map(seq_pkg).collect();
    let got = target.pushed();
    assert_eq!(got.len(), want.len());
    let first = got.iter().zip(&want).position(|(a, b)| a != b);
    let misplaced = got.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert!(
        first.is_none(),
        "{} children: out of order from position {:?}, {} of {} packages misplaced",
        children,
        first,
        misplaced,
        n
    );
}

/// Spreader with ordering: one sender takes the turns, so with two targets each holds every
/// other package in strict order, for any number of children. The targets answer slowly so
/// several children are popping while the sender delivers.
fn strict_order_spreader(test: &str, children: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let targets = [
        SlowTarget::start(Duration::from_millis(2)),
        SlowTarget::start(Duration::from_millis(2)),
    ];
    let n: u64 = 2000;
    let clients = [
        ctx.client_yaml_at("t0", "rmtest:slow0", targets[0].port),
        ctx.client_yaml_at("t1", "rmtest:slow1", targets[1].port),
    ];
    let cfg = ctx.config(&src, children, "spreader", ORDERING_1S, &clients);
    let mut p = ctx.spawn(&cfg);
    for start in (0..n).step_by(50) {
        let batch: Vec<String> = (start..start + 50).map(seq_pkg).collect();
        ctx.push(&src, &batch);
        thread::sleep(Duration::from_millis(50));
    }
    ctx.wait_for("every package at a slow target", |c| {
        c.llen(&src) == 0 && targets[0].count() + targets[1].count() == n as usize
    });
    p.stop(&ctx.pid_file());
    for (i, t) in targets.iter().enumerate() {
        let want: Vec<String> = (0..n).skip(i).step_by(2).map(seq_pkg).collect();
        let got = t.pushed();
        let first = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(
            got.len(),
            want.len(),
            "{} children: target t{}",
            children,
            i
        );
        assert!(
            first.is_none(),
            "{} children: target t{} out of turn or order from position {:?}",
            children,
            i,
            first
        );
    }
}

#[test]
fn ordering_strict_order_spreader_two_children() {
    strict_order_spreader("ordering_strict_spr_2", 2);
}

#[test]
fn ordering_strict_order_spreader_eight_children() {
    strict_order_spreader("ordering_strict_spr_8", 8);
}

#[test]
fn ordering_strict_order_two_children() {
    strict_order("ordering_strict_2", 2);
}

#[test]
fn ordering_strict_order_eight_children() {
    strict_order("ordering_strict_8", 8);
}
