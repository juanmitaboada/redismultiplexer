// Connection handshake: what the program sends when it connects.
mod common;
use common::*;

/// Error replies the server has sent since it started (INFO stats, Redis 6.2 or later).
fn error_replies(ctx: &mut Ctx) -> u64 {
    let info: String = ctx.cmd(&["INFO", "stats"]);
    info.lines()
        .find_map(|l| l.strip_prefix("total_error_replies:"))
        .expect("total_error_replies in INFO stats (Redis 6.2 or later)")
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn connect_sends_no_client_setinfo() {
    // M3R-2: redis 0.32 sends CLIENT SETINFO on every connection unless the crate feature
    // `disable-client-setinfo` is on. Servers before 7.2 answer two errors per connection,
    // and a peer that closes on an unknown command breaks the connect; 1.0.8 sent nothing
    // but AUTH. Before 7.2 this test sees the error replies, from 7.2 the lib-name.
    // This file holds one test so no other test of the suite runs beside it.
    let mut ctx = Ctx::new("handshake");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&src, &packages("h", 5));
    let cfg = ctx.config(&src, 2, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let before = error_replies(&mut ctx);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 5)]);
    let clients: String = ctx.cmd(&["CLIENT", "LIST"]);
    p.stop(&ctx.pid_file());
    let after = error_replies(&mut ctx);
    assert!(!clients.contains("lib-name=redis-rs"), "{}", clients);
    assert_eq!(after - before, 0, "error replies while the program ran");
}
