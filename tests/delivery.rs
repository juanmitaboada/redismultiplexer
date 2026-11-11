//! Statistics on the delivery paths (stage 3: workers deliver directly, ordered packages go
//! through the sender). Failing targets are in failures.rs.

mod common;

use common::*;

/// `total_*` of one status file write
fn totals(json: &str) -> [u64; 3] {
    ["total_in", "total_out", "total_drop"].map(|key| {
        let at = json.find(&format!("\"{}\":", key)).expect(key) + key.len() + 3;
        json[at..]
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap()
    })
}

/// The status file accounts for every package, with the workers' and the sender's
/// counters summed by main (per-worker atomic statistics): the periodic write after 10 s
/// plus the last write, made after the stop (M3R-12), hold every package once.
fn status_counts(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let real: Vec<String> = (0..cell.targets())
        .map(|i| ctx.key(&format!("t{}", i)))
        .collect();
    let clients: Vec<String> = real
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    let input: Vec<String> = (0..1000).map(seq_pkg).collect();
    ctx.push(&src, &input);
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("every package delivered", |c| {
        let got: usize = real.iter().map(|k| c.llen(k)).sum();
        c.llen(&src) == 0 && got == 1000
    });
    let status = ctx.dir.join("rm.status");
    // Written every 10 s from the start
    ctx.wait_for("status file", |_| status.exists());
    let first = totals(&std::fs::read_to_string(&status).unwrap());
    p.stop(&ctx.pid_file());
    let last = totals(&std::fs::read_to_string(&status).unwrap());
    let sum = [0, 1, 2].map(|i| first[i] + last[i]);
    assert_eq!(
        sum,
        [1000, 1000, 0],
        "in, out, drop: {:?} + {:?}",
        first,
        last
    );
    if cell.ordering {
        assert!(p.stdout().contains("Queue: 0 regs"), "{}", p.stdout());
    }
}

matrix!(status_file_counts_every_package, status_counts, 2);
