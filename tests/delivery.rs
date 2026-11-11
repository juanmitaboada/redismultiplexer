//! Statistics and a failing target on the delivery paths (stage 3: workers deliver
//! directly, ordered packages go through the sender).

mod common;

use common::*;

/// The status file's counts of the first 10 s interval account for every package, with the
/// workers' and the sender's counters summed by main (per-worker atomic statistics).
fn status_counts(test: &str, ordering: bool) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let input: Vec<String> = (0..1000).map(seq_pkg).collect();
    ctx.push(&src, &input);
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(
        &src,
        2,
        "replicant",
        extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 1000)]);
    let status = ctx.dir.join("rm.status");
    // Written every 10 s from the start
    ctx.wait_for("status file", |_| status.exists());
    p.stop(&ctx.pid_file());
    let json = std::fs::read_to_string(&status).unwrap();
    for (key, want) in [("total_in", 1000), ("total_out", 1000), ("total_drop", 0)] {
        assert!(
            json.contains(&format!("\"{}\":{}", key, want)),
            "{} != {} in {}",
            key,
            want,
            json
        );
    }
    if ordering {
        assert!(p.stdout().contains("Queue: 0 regs"), "{}", p.stdout());
    }
}

#[test]
fn status_file_counts_every_package() {
    status_counts("status_counts", false);
}

#[test]
fn status_file_counts_every_package_with_ordering() {
    status_counts("status_counts_o", true);
}

/// A target whose key is not a list answers every command with an error: the other target
/// still gets every package, from the workers and from the sender.
fn failing_target(test: &str, ordering: bool) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let bad = ctx.key("bad");
    let _: () = ctx.cmd(&["SET", &bad, "not a list"]);
    let input: Vec<String> = (0..200).map(seq_pkg).collect();
    ctx.push(&src, &input);
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(
        &src,
        2,
        "replicant",
        extra,
        &[
            ctx.client_yaml("t0", &t0, ""),
            ctx.client_yaml("bad", &bad, ""),
        ],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 200)]);
    p.stop(&ctx.pid_file());
    let mut got = ctx.list(&t0);
    got.sort();
    let mut want = input.clone();
    want.sort();
    assert_eq!(got, want);
    assert!(p.stderr().contains("WRONGTYPE"), "{}", p.stderr());
}

#[test]
fn failing_target_does_not_stop_the_other() {
    failing_target("failing_target", false);
}

#[test]
fn failing_target_does_not_stop_the_other_with_ordering() {
    failing_target("failing_target_o", true);
}
