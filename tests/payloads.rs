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

/// Bytes that are not UTF-8 arrive byte for byte on every delivery path (project rule,
/// M3R-12): worker and sender, one child and several, replicant and spreader.
fn binary_on_every_path(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let n = 60u64;
    let pkg = |i: u64| -> Vec<u8> {
        let mut p = format!("{{\"ts\": {}, \"v\": ", i).into_bytes();
        p.extend_from_slice(&[0xff, 0xfe, 0x00, b'\r', b'\n', 0xc3, i as u8]);
        p.push(b'}');
        p
    };
    let input: Vec<Vec<u8>> = (0..n).map(pkg).collect();
    let refs: Vec<&[u8]> = input.iter().map(|p| p.as_slice()).collect();
    ctx.push_bytes(&src, &refs);
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        ctx.client_yaml("t1", &t1, ""),
    ];
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    let total = if cell.spreader { n } else { 2 * n } as usize;
    ctx.wait_for("every package delivered", |c| {
        c.llen(&src) == 0 && c.llen(&t0) + c.llen(&t1) == total
    });
    p.stop(&ctx.pid_file());
    let (got0, got1) = (ctx.list_bytes(&t0), ctx.list_bytes(&t1));
    if cell.in_order() {
        assert_ts_increasing("t0", &got0);
        assert_ts_increasing("t1", &got1);
    }
    let mut want = input.clone();
    want.sort();
    if cell.spreader {
        let mut all = [got0, got1].concat();
        all.sort();
        assert_eq!(all, want, "spreader: not every package once, byte for byte");
    } else {
        for (name, mut got) in [("t0", got0), ("t1", got1)] {
            got.sort();
            assert_eq!(got, want, "{}: not every package once, byte for byte", name);
        }
    }
    assert!(!p.stderr().contains("UTF8"), "{}", p.stderr());
}

matrix!(binary_every_path, binary_on_every_path, 4);
