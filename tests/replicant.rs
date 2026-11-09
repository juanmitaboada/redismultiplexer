//! Replicant mode: every package goes to every client.

mod common;

use common::*;

#[test]
fn replicant_one_client() {
    let mut ctx = Ctx::new("replicant_one");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let all = packages("p", 100);
    ctx.push(&src, &all);
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 100)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), all);
}

#[test]
fn replicant_two_clients() {
    let mut ctx = Ctx::new("replicant_two");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let mut all = packages("p", 200);
    ctx.push(&src, &all);
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        ctx.client_yaml("t1", &t1, ""),
    ];
    let cfg = ctx.config(&src, 2, "replicant", "", &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 200), (&t1, 200)]);
    p.stop(&ctx.pid_file());
    all.sort();
    for t in [&t0, &t1] {
        let mut got = ctx.list(t);
        got.sort();
        assert_eq!(got, all, "target {}", t);
    }
}
