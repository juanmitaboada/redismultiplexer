//! The delivery paths (stage 3: workers deliver directly, ordered packages go through the
//! sender): every package and the statistics, over the matrix. Failing targets are in
//! failures.rs.

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
    // Written every 10 s from the start. The write is not atomic (truncate, then write;
    // atomic is stage 4, M3-e): wait for a complete one, not just for the file
    let mut json = String::new();
    ctx.wait_for("a complete status file", |_| {
        json = std::fs::read_to_string(&status).unwrap_or_default();
        json.trim_end().ends_with('}') && json.contains("\"total_drop\":")
    });
    let first = totals(&json);
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

/// Every target gets every package on every delivery path (project rule, M3R-12), in
/// source order where the cell keeps it.
fn replicant_on_every_path(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let n = 300u64;
    let all: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &all);
    let clients = [
        ctx.client_yaml("t0", &t0, ""),
        ctx.client_yaml("t1", &t1, ""),
    ];
    let cfg = ctx.config(&src, cell.children, "replicant", cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, n as usize), (&t1, n as usize)]);
    p.stop(&ctx.pid_file());
    for t in [&t0, &t1] {
        let got = ctx.list(t);
        if cell.in_order() {
            assert_eq!(got, all, "target {}", t);
        } else {
            let (mut got, mut want) = (got, all.clone());
            got.sort();
            want.sort();
            assert_eq!(got, want, "target {}", t);
        }
    }
}

/// Every package on exactly one target on every delivery path (project rule, M3R-12).
/// Where one thread takes the turns (one child, or ordering: one sender) the turn is exact:
/// target i gets packages i, i + 3, ... in order.
fn spreader_on_every_path(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let keys: Vec<String> = (0..3).map(|i| ctx.key(&format!("t{}", i))).collect();
    let n = 300u64;
    let all: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &all);
    let clients: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    let cfg = ctx.config(&src, cell.children, "spreader", cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("every package on a target", |c| {
        c.llen(&src) == 0 && keys.iter().map(|k| c.llen(k)).sum::<usize>() == n as usize
    });
    p.stop(&ctx.pid_file());
    let lists: Vec<Vec<String>> = keys.iter().map(|k| ctx.list(k)).collect();
    if cell.in_order() {
        for (i, l) in lists.iter().enumerate() {
            let want: Vec<String> = all.iter().skip(i).step_by(3).cloned().collect();
            assert_eq!(l, &want, "target t{}", i);
        }
    }
    let mut got = lists.concat();
    got.sort();
    let mut want = all;
    want.sort();
    assert_eq!(got, want, "not every package exactly once");
}

/// Replicant and spreader on every delivery path, one package set per cell
fn every_package_on_every_path(test: &str, cell: Cell) {
    if cell.spreader {
        spreader_on_every_path(test, cell);
    } else {
        replicant_on_every_path(test, cell);
    }
}

matrix!(every_path, every_package_on_every_path, 4);
