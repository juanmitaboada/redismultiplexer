//! Payload encoding.

mod common;

use common::*;

#[test]
fn non_utf8_payload_is_lost_until_m3() {
    // A-C1: a non-UTF-8 payload fails from_utf8, is lost and makes the worker
    // reconnect. Expected to change in M3 (bytes payload end to end).
    let mut ctx = Ctx::new("non_utf8");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push_bytes(&src, &[b"a", &[0xff, 0xfe], b"b"]);
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 2)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), vec!["a", "b"]);
    assert!(
        p.stderr().contains("Couldn't decode to UTF8"),
        "{}",
        p.stderr()
    );
}
