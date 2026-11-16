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
    // Written every 10 s from the start, whole (a file renamed over it, M3-e): read once
    ctx.wait_for("the status file", |_| status.exists());
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

/// The breakdown of `drop` (M3-e = ea, D4d-3 = a), over the matrix: packages the root
/// filter refuses are `filtered`; those only a stuck target accepts are `stuck`; those only a
/// target that is down accepts are `failed`; the rest delivered. A package that both the
/// stuck and the down target accept is `failed` (failed wins, D4d-3 = a), and one that only
/// a target answering errors accepts is `failed` (M3R-39). The last write (after the stop) carries
/// the counts since the start, the stuck and the down targets, the version.
fn status_breakdown(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let healthy = ctx.key("t0");
    let full = ctx.key("t1");
    ctx.push(&full, &packages("old", 10));
    let refusing = ctx.key("t3");
    let _: () = ctx.cmd(&["SET", &refusing, "not a list"]);
    let down = closed_port();
    let limits = "    timelimit: 1\n    checklimit: 1\n    softlimit: 2\n    hardlimit: 5\n";
    let clients = [
        ctx.client_yaml("t0", &healthy, "    filter: '^k'\n"),
        ctx.client_yaml("t1", &full, &format!("    filter: '^(s|b)'\n{}", limits)),
        format!(
            "  - name: 't2'\n    ssl: false\n    hostname: '127.0.0.1'\n    port: {}\n    password: ''\n    channel: 'rmtest:down'\n    filter: '^(d|b)'\n{}",
            down, SHORT_TIMEOUTS
        ),
        ctx.client_yaml("t3", &refusing, "    filter: '^e'\n"),
    ];
    // k: delivered; s: only the stuck target; d: only the down one; x: the root filter;
    // b: the stuck and the down target; e: only the target that answers errors
    let mut input = Vec::new();
    for (i, kind) in ["k", "s", "d", "x", "k", "k", "b", "e"]
        .iter()
        .cycle()
        .take(800)
        .enumerate()
    {
        input.push(format!("{}{}", kind, seq_pkg(i as u64)));
    }
    // 100 more for the root filter, so no two counts of the breakdown are equal
    input.extend((800..900).map(|i| format!("x{}", seq_pkg(i))));
    ctx.push(&src, &input);
    let root = format!("filter: '^(k|s|d|b|e)'\n{}", cell.extra());
    let cfg = ctx.config(&src, cell.children, cell.mode(), &root, &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("the source drained", |c| {
        c.llen(&src) == 0 && c.llen(&healthy) == 300
    });
    p.stop(&ctx.pid_file());
    let status: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ctx.dir.join("rm.status")).unwrap()).unwrap();
    let since = &status["since_start"];
    let n = |k: &str| {
        since[k]
            .as_u64()
            .unwrap_or_else(|| panic!("{} in {}", k, status))
    };
    assert_eq!(
        [
            n("in"),
            n("out"),
            n("drop"),
            n("filtered"),
            n("stuck"),
            n("failed")
        ],
        [900, 300, 600, 200, 100, 300],
        "{}",
        status
    );
    assert_eq!(ctx.llen(&full), 10, "the stuck target took packages");
    let names = |k: &str| -> Vec<String> {
        status[k]
            .as_array()
            .unwrap_or_else(|| panic!("{} in {}", k, status))
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("stuck_targets"), vec![format!("t1:{}", full)]);
    assert_eq!(names("down_targets"), vec!["t2:rmtest:down".to_string()]);
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["queue"], 0);
    let still: String = ctx.cmd(&["GET", &refusing]);
    assert_eq!(still, "not a list", "the refusing target took packages");
    // The last interval's breakdown sums to its drop, as every interval's does
    let t = |k: &str| status[k].as_u64().unwrap();
    assert_eq!(
        t("total_filtered") + t("total_stuck") + t("total_failed"),
        t("total_drop")
    );
    // Each total is its own counter (M3R-39): the last interval's share of the run, its
    // rate is that total over the interval, and a run inside one interval (the usual case
    // here: the 10 s between writes) gives the counts since the start
    let rate = |k: &str| status[k].as_f64().unwrap();
    let one_interval = t("total_in") == n("in");
    for k in ["filtered", "stuck", "failed"] {
        let total = t(&format!("total_{}", k));
        assert!(total <= n(k), "total_{} above the run's: {}", k, status);
        let (lhs, rhs) = (rate(k) * t("total_in") as f64, rate("in") * total as f64);
        assert!(
            (lhs - rhs).abs() <= 1e-6 * lhs.max(1.0),
            "{}: {}",
            k,
            status
        );
        if one_interval {
            assert_eq!(total, n(k), "total_{}: {}", k, status);
        }
    }
}

matrix!(status_drop_breakdown, status_breakdown, 2);

/// The status file's `queue` is what the ordering buffer holds when it is written (M3R-39):
/// a buffer time far above the 10 s between writes keeps every package in it
#[test]
fn status_queue_counts_the_held_packages() {
    let mut ctx = Ctx::new("status_queue_held");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let n = 40;
    let input: Vec<String> = (0..n).map(seq_pkg).collect();
    let extra =
        "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 120\nordering_limit: 200\n";
    let cfg = ctx.config(
        &src,
        2,
        "replicant",
        extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &input);
    ctx.wait_for("the source drained", |c| c.llen(&src) == 0);
    let status = ctx.dir.join("rm.status");
    let queue = || -> u64 {
        std::fs::read_to_string(&status)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["queue"].as_u64())
            .unwrap_or(0)
    };
    // The next write (at most 10 s away) sees the packages held
    let start = std::time::Instant::now();
    while queue() != n && start.elapsed() < std::time::Duration::from_secs(25) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert_eq!(
        queue(),
        n,
        "queue in {:?}",
        std::fs::read_to_string(&status)
    );
    assert_eq!(ctx.llen(&t0), 0, "released before the buffer time");
    p.stop(&ctx.pid_file());
    assert_eq!(
        ctx.list(&t0),
        input,
        "not every held package at the stop, in order"
    );
    assert_eq!(queue(), 0);
}

/// Where the status file cannot be replaced whole (a directory holds `<status>.tmp`) it is
/// written in place with one warning, the first time only (D4d-4 = a, M3R-39): the write
/// of the 10 s tick and the one of the stop (a package in between), one warning
#[test]
fn status_in_place_warning_once() {
    let mut ctx = Ctx::new("status_in_place_once");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    std::fs::create_dir(ctx.dir.join("rm.status.tmp")).unwrap();
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    let status = ctx.dir.join("rm.status");
    ctx.wait_for("the first status write", |_| status.exists());
    let first = std::fs::read_to_string(&status).unwrap();
    // A package after it, so the stop's write differs from the first
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    p.stop(&ctx.pid_file());
    let last = std::fs::read_to_string(&status).unwrap();
    assert_ne!(first, last, "no second write");
    let warnings = p.stderr().matches("is written in place").count();
    assert_eq!(warnings, 1, "{}", p.stderr());
}

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
