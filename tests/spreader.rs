//! Spreader mode: round robin over the clients.

mod common;

use common::*;

#[test]
fn spreader_three_clients_round_robin() {
    let mut ctx = Ctx::new("spreader_rr");
    let src = ctx.key("src");
    let keys: Vec<String> = (0..3).map(|i| ctx.key(&format!("t{}", i))).collect();
    let all = packages("p", 99);
    ctx.push(&src, &all);
    let clients: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    let cfg = ctx.config(&src, 1, "spreader", "", &clients);
    let mut p = ctx.spawn(&cfg);
    let expected: Vec<(&str, usize)> = keys.iter().map(|k| (k.as_str(), 33)).collect();
    ctx.wait_drained(&src, &expected);
    p.stop(&ctx.pid_file());
    for (i, k) in keys.iter().enumerate() {
        let want: Vec<String> = all.iter().skip(i).step_by(3).cloned().collect();
        assert_eq!(ctx.list(k), want, "target t{}", i);
    }
}

#[test]
fn spreader_skips_stuck_client() {
    let mut ctx = Ctx::new("spreader_stuck");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let limits = "    timelimit: 1\n    checklimit: 1\n    softlimit: 5\n    hardlimit: 10\n";
    let clients = [
        ctx.client_yaml("t0", &t0, limits),
        ctx.client_yaml("t1", &t1, ""),
    ];
    let cfg = ctx.config(&src, 1, "spreader", "", &clients);
    let mut p = ctx.spawn(&cfg);
    let all = packages("p", 20);
    ctx.push(&src, &all);
    ctx.wait_drained(&src, &[(&t1, 20)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), filler);
    assert_eq!(ctx.list(&t1), all);
}
