//! Stop under load with more than one child (M3R-3).
//!
//! The one-child cases are in lifecycle.rs (ordering off) and ordering.rs (ordering on).

mod common;

use common::*;
use std::thread;
use std::time::{Duration, Instant};

/// Packages that may leave the source after the stop signal (M3F-13): about 10 times what
/// the session measured (19-310 in 8 runs, 8 children); 1.0.8 took 291-32060.
const LIMIT: usize = 3_000;

#[test]
fn stop_under_load_two_children() {
    stop_under_load("stop_load_c2", 2, false, Duration::from_millis(500));
}

#[test]
fn stop_under_load_two_children_with_ordering() {
    // After the first release, so packages are both delivered and held when the stop comes
    stop_under_load("stop_load_c2_ordering", 2, true, Duration::from_millis(300));
}

#[test]
fn stop_under_load_four_children() {
    stop_under_load("stop_load_c4", 4, false, Duration::from_millis(500));
}

#[test]
fn stop_under_load_four_children_with_ordering() {
    stop_under_load("stop_load_c4_ordering", 4, true, Duration::from_millis(300));
}

// SIGTERM (`systemctl stop`) and SIGHUP stop as SIGINT does (M3-l = la, M3F-3): before,
// SIGTERM killed the process at once, losing the ordering buffer and leaving the pid file.

#[test]
fn stop_under_load_sigterm_two_children() {
    stop_under_load_with(
        "stop_load_term",
        2,
        false,
        Duration::from_millis(500),
        "TERM",
    );
}

#[test]
fn stop_under_load_sigterm_two_children_with_ordering() {
    stop_under_load_with(
        "stop_load_term_o",
        2,
        true,
        Duration::from_millis(300),
        "TERM",
    );
}

#[test]
fn stop_under_load_sighup_two_children() {
    stop_under_load_with("stop_load_hup", 2, false, Duration::from_millis(500), "HUP");
}

#[test]
fn stop_under_load_sighup_two_children_with_ordering() {
    stop_under_load_with(
        "stop_load_hup_o",
        2,
        true,
        Duration::from_millis(300),
        "HUP",
    );
}

#[test]
fn sighup_is_ignored_when_started_under_nohup() {
    // M3F-14 = 14b: SIGHUP stops the program (M3-l), except when it started with SIGHUP
    // ignored (nohup), which keeps it running as before; SIGINT still stops it.
    let mut ctx = Ctx::new("stop_nohup");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(&src, 2, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn_hup_ignored(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    p.signal("HUP");
    thread::sleep(Duration::from_millis(1500));
    ctx.push(&src, &[seq_pkg(2)]);
    ctx.wait_drained(&src, &[(&t0, 2)]);
    p.stop(&ctx.pid_file());
}

#[test]
fn stop_is_prompt() {
    // No fixed sleeps on the way out: an idle program (each child in a BLPOP of at most
    // 1 s) stops within 2 s of the signal. 1.0.8 slept 2 s in main and 1.5 s per child.
    let mut ctx = Ctx::new("stop_prompt");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(
        &src,
        4,
        "replicant",
        ORDERING_1S,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    let start = Instant::now();
    p.stop(&ctx.pid_file());
    let took = start.elapsed();
    assert!(took < Duration::from_secs(2), "stop took {:?}", took);
}

#[test]
fn stop_takes_little_after_the_signal() {
    // M3F-13: with 8 children and ordering, 1.0.8 kept popping for up to a second after the
    // signal (main noticed it on its next 1 s turn), enough to empty a busy source. Now the
    // signal sets the stop flag the children read after each package: what leaves the
    // source after the signal is the packages in flight plus what is popped while the
    // signal is delivered.
    let mut ctx = Ctx::new("stop_little");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let n = 400_000u64;
    for start in (0..n).step_by(10_000) {
        let batch: Vec<String> = (start..start + 10_000).map(seq_pkg).collect();
        ctx.push(&src, &batch);
    }
    let cfg = ctx.config(
        &src,
        8,
        "replicant",
        ORDERING_1S,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("first package delivered", |c| c.llen(&t0) > 0);
    thread::sleep(Duration::from_millis(200));
    let before = ctx.llen(&src);
    p.stop(&ctx.pid_file());
    let after = ctx.llen(&src);
    assert!(before > 0, "the source drained before the signal");
    let taken = before - after;
    assert!(
        taken <= LIMIT,
        "{} packages left the source after the signal",
        taken
    );
}
