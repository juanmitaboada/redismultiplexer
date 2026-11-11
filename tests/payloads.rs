//! Payload encoding.

mod common;

use common::*;

#[test]
fn non_utf8_payload_is_delivered() {
    // A-C1 fixed: payloads are bytes end to end. Before 1.1 a non-UTF-8 payload was lost
    // and made the worker reconnect.
    let mut ctx = Ctx::new("non_utf8");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let binary: &[u8] = &[0xff, 0xfe, 0x00, b'x', 0xc3];
    ctx.push_bytes(&src, &[b"a", binary, b"b"]);
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 3)]);
    p.stop(&ctx.pid_file());
    assert_eq!(
        ctx.list_bytes(&t0),
        vec![b"a".to_vec(), binary.to_vec(), b"b".to_vec()]
    );
    assert!(!p.stderr().contains("UTF8"), "{}", p.stderr());
}

#[test]
fn non_utf8_payload_through_filters_and_ordering() {
    // Root filter, client filter with replacement and ordering all read the payload as bytes.
    let mut ctx = Ctx::new("non_utf8_filters");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let late: &[u8] = b"k:{\"ts\": 2,}\xff\xfe";
    let early: &[u8] = b"k:{\"ts\": 1,}\x80";
    let other: &[u8] = b"z:{\"ts\": 0,}\xff";
    ctx.push_bytes(&src, &[late, early, other]);
    let extra = "filter: '^k:'\nordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 1\nordering_limit: 64\n";
    let client = ctx.client_yaml("t0", &t0, "    filter: '^k:'\n    filter_replace: 'K='\n");
    let cfg = ctx.config(&src, 1, "replicant", extra, &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 2)]);
    p.stop(&ctx.pid_file());
    assert_eq!(
        ctx.list_bytes(&t0),
        vec![
            b"K={\"ts\": 1,}\x80".to_vec(),
            b"K={\"ts\": 2,}\xff\xfe".to_vec()
        ]
    );
}
