//! Queue limits: hardlimit / softlimit and deleteblock.

mod common;

use common::*;
use std::thread;
use std::time::Duration;

const LIMITS: &str = "    timelimit: 1\n    checklimit: 1\n    softlimit: 5\n    hardlimit: 10\n";

#[test]
fn hardlimit_stall_drops_and_release() {
    // Intended behaviour: while a client is stuck its packages are dropped, so
    // queues can never fill the machine. Delivery resumes under softlimit.
    let mut ctx = Ctx::new("hardlimit");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, LIMITS)],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &packages("lost", 20));
    ctx.wait_drained(&src, &[(&t0, 10)]);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(ctx.list(&t0), filler, "stuck client received packages");

    // Free the target below softlimit and wait for the idle re-check.
    ctx.ltrim(&t0, 8, -1);
    ctx.wait_for("client freed", |_| p.stderr().contains("freed!"));
    let fresh = packages("new", 5);
    ctx.push(&src, &fresh);
    ctx.wait_drained(&src, &[(&t0, 7)]);
    p.stop(&ctx.pid_file());
    let mut want = filler[8..].to_vec();
    want.extend(fresh);
    assert_eq!(ctx.list(&t0), want);
    assert!(p.stderr().contains("stuck!"));
}

#[test]
fn deleteblock_trims_oldest() {
    let mut ctx = Ctx::new("deleteblock");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let limits = format!("{}    deleteblock: 4\n", LIMITS);
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, &limits)],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &["new".to_string()]);
    ctx.wait_drained(&src, &[(&t0, 7)]);
    p.stop(&ctx.pid_file());
    let mut want = filler[4..].to_vec();
    want.push("new".to_string());
    assert_eq!(ctx.list(&t0), want);
}
