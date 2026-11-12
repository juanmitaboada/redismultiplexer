//! A target that fails, closes or cannot be reached (M3R-12, L3b): the other targets keep
//! receiving every package, with ordering and without, and nothing is lost uncounted at
//! the stop. Every test runs over the matrix ordering on and off, replicant and spreader,
//! one child and several.

mod common;

use common::*;
use std::thread;
use std::time::{Duration, Instant};

/// Packages `0..n` fed at about 1000 per second from a thread, so the run is long enough
/// for a target to go down in the middle of it.
fn feed(ctx: &Ctx, src: &str, n: u64) -> thread::JoinHandle<()> {
    let (url, src) = (ctx.ep.url.clone(), src.to_string());
    thread::spawn(move || {
        let mut con = redis::Client::open(url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        for start in (0..n).step_by(50) {
            let batch: Vec<String> = (start..(start + 50).min(n)).map(seq_pkg).collect();
            let _: usize = redis::Commands::rpush(&mut con, &src, batch).unwrap();
            thread::sleep(Duration::from_millis(50));
        }
    })
}

/// Every package `0..n` is on exactly one of `lists` (spreader) or on each of them
/// (replicant), in order where the cell keeps the order.
fn assert_all(cell: Cell, lists: &[Vec<String>], n: u64) {
    let want: Vec<String> = (0..n).map(seq_pkg).collect();
    for l in lists {
        if cell.in_order() {
            let ts: Vec<u64> = l
                .iter()
                .map(|p| p[7..p.find(',').unwrap()].parse().unwrap())
                .collect();
            assert!(
                ts.windows(2).all(|w| w[0] < w[1]),
                "a target is out of order"
            );
        }
    }
    if cell.spreader {
        let mut all: Vec<String> = lists.concat();
        all.sort();
        let mut w = want.clone();
        w.sort();
        assert_eq!(
            all.len(),
            w.len(),
            "spreader: not every package exactly once"
        );
        assert_eq!(all, w, "spreader: not every package exactly once");
    } else {
        for l in lists {
            let mut got = l.clone();
            got.sort();
            let mut w = want.clone();
            w.sort();
            assert_eq!(got, w, "replicant: a healthy target misses packages");
        }
    }
}

/// A second target takes part and then goes down for good in the middle of the run
/// (connections closed, port refusing): the healthy target keeps receiving. Spreader: the
/// packages the closed target took before it went down plus the healthy target's are every
/// package once. Before M3R-12, with ordering, the sender stopped delivering to every
/// target once one failed.
fn target_closes_mid_run(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let other = SlowTarget::start(Duration::ZERO);
    let n = 3000;
    let cfg = ctx.config(
        &src,
        cell.children,
        cell.mode(),
        cell.extra(),
        &[
            ctx.client_yaml("t0", &t0, ""),
            ctx.client_yaml_at("other", "rmtest:other", other.port),
        ],
    );
    let mut p = ctx.spawn(&cfg);
    let producer = feed(&ctx, &src, n);
    ctx.wait_for("the other target took packages", |_| other.count() > 0);
    thread::sleep(Duration::from_millis(500));
    other.close();
    producer.join().unwrap();
    let taken_by_other = other.pushed();
    let need = if cell.spreader {
        n as usize - taken_by_other.len()
    } else {
        n as usize
    };
    ctx.wait_drained(&src, &[(&t0, need)]);
    p.stop(&ctx.pid_file());
    let t0_list = ctx.list(&t0);
    if cell.spreader {
        assert_all(cell, &[t0_list, taken_by_other], n);
    } else {
        assert_all(cell, &[t0_list], n);
    }
}

/// A target refuses connections from the start: the healthy target receives everything,
/// and the refused one is tried at most once a second by each worker and the sender.
fn target_unreachable_from_start(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let real: Vec<String> = (0..cell.targets())
        .map(|i| ctx.key(&format!("t{}", i)))
        .collect();
    let mut clients: Vec<String> = real
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    clients.insert(0, ctx.client_yaml_at("down", "rmtest:down", closed_port()));
    let n = 2000;
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let start = Instant::now();
    let mut p = ctx.spawn(&cfg);
    let producer = feed(&ctx, &src, n);
    producer.join().unwrap();
    ctx.wait_for("every package at the healthy targets", |c| {
        let got: usize = real.iter().map(|k| c.llen(k)).sum();
        c.llen(&src) == 0
            && if cell.spreader {
                got == n as usize
            } else {
                got == n as usize * real.len()
            }
    });
    p.stop(&ctx.pid_file());
    let secs = start.elapsed().as_secs() + 1;
    let lists: Vec<Vec<String>> = real.iter().map(|k| ctx.list(k)).collect();
    assert_all(cell, &lists, n);
    // Each thread that delivers (the workers, and the sender with ordering) tries the
    // refused target at most once a second, plus its first attempt
    let threads = cell.children as u64 + cell.ordering as u64;
    let attempts = p
        .stderr()
        .matches("Error while connecting to target '127.0.0.1")
        .count();
    assert!(attempts > 0, "the refused target was never tried");
    assert!(
        attempts as u64 <= threads * (secs + 1),
        "{} connect attempts in {} s by {} threads",
        attempts,
        secs,
        threads
    );
}

/// A target whose key is not a list answers every command with an error but keeps its
/// connection: the other targets still get every package, from the workers and from the
/// sender.
fn wrongtype_target(test: &str, cell: Cell) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let bad = ctx.key("bad");
    let _: () = ctx.cmd(&["SET", &bad, "not a list"]);
    let n = 200;
    let input: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &input);
    let cfg = ctx.config(
        &src,
        cell.children,
        cell.mode(),
        cell.extra(),
        &[
            ctx.client_yaml("bad", &bad, ""),
            ctx.client_yaml("t0", &t0, ""),
        ],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, n as usize)]);
    p.stop(&ctx.pid_file());
    assert_all(cell, &[ctx.list(&t0)], n);
    assert!(p.stderr().contains("WRONGTYPE"), "{}", p.stderr());
}

/// Stop under load with a target down the whole run: every package is delivered to the
/// healthy targets or left in the source, exactly once.
fn stop_with_a_target_down(test: &str, cell: Cell) {
    stop_under_load_cell(test, cell, true, Duration::from_millis(300), "INT");
}

/// After the signal, packages keep arriving while the workers may still be inside a BLPOP
/// of up to 1 s: whatever a worker pops then must still be delivered (the sender flushes
/// only after every worker has left, M3R-15) or stay in the source. Before the fix a sender
/// told to finish before the workers were joined lost such packages.
fn push_after_the_signal(test: &str, cell: Cell) {
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
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut next = 0u64;
    // Two runs: each must account for every package
    for _ in 0..2 {
        let mut p = ctx.spawn(&cfg);
        ctx.push(&src, &[seq_pkg(next)]);
        next += 1;
        ctx.wait_for("first package delivered", |c| {
            real.iter().map(|k| c.llen(k)).sum::<usize>() == next as usize
        });
        // Idle now: every worker waits in a BLPOP
        p.signal("INT");
        let until = Instant::now() + Duration::from_millis(1200);
        while Instant::now() < until {
            ctx.push(&src, &[seq_pkg(next)]);
            next += 1;
            thread::sleep(Duration::from_millis(20));
        }
        let st = p.wait_exit();
        assert!(st.success(), "exit status {:?}", st);
        // What is left in the source goes to the next run
        let left: Vec<String> = ctx.list(&src);
        let delivered: Vec<String> = real.iter().flat_map(|k| ctx.list(k)).collect();
        assert_eq!(
            delivered.len() + left.len(),
            next as usize,
            "delivered {} + left {} != pushed {}",
            delivered.len(),
            left.len(),
            next
        );
        let mut all: Vec<String> = delivered.into_iter().chain(left).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), next as usize, "a package delivered twice");
        // Start the next run from an empty source: move what is left into a target
        let rest = ctx.list(&src);
        if !rest.is_empty() {
            ctx.ltrim(&src, 1, 0);
            ctx.push(&real[0], &rest);
        }
    }
}

/// A target that is not refused but never answers, in the two places a thread can wait
/// for it: the connect (`blackhole`: SYNs dropped, as behind a firewall that drops) and the
/// reply (`silent`: the connection is accepted, nothing ever comes back). The healthy target
/// still receives every package: the connect and the command give up after their timeouts
/// (here the short ones of SHORT_TIMEOUTS), then the target waits out its backoff (M3R-12
/// X1b; until then a thread waited for the kernel's connect timeout, about 2 min, or for
/// ever).
fn unanswering_target(test: &str, cell: Cell, port: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let real: Vec<String> = (0..cell.targets())
        .map(|i| ctx.key(&format!("t{}", i)))
        .collect();
    let mut clients: Vec<String> = real
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    // Short timeouts (S4-6): the defaults are asserted in timeouts.rs
    clients.insert(
        0,
        ctx.client_yaml_at("mute", "rmtest:mute", port) + SHORT_TIMEOUTS,
    );
    let n = 200;
    let input: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &input);
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("every package at the healthy targets", |c| {
        let got: usize = real.iter().map(|k| c.llen(k)).sum();
        c.llen(&src) == 0
            && got
                == if cell.spreader {
                    n as usize
                } else {
                    n as usize * real.len()
                }
    });
    p.stop(&ctx.pid_file());
    let lists: Vec<Vec<String>> = real.iter().map(|k| ctx.list(k)).collect();
    assert_all(cell, &lists, n);
}

fn blackhole_target(test: &str, cell: Cell) {
    let hole = blackhole();
    unanswering_target(test, cell, hole.port);
}

fn silent_target(test: &str, cell: Cell) {
    let mute = SilentTarget::start();
    unanswering_target(test, cell, mute.port);
}

/// A blackholed target (its connects hang until the connect timeout) does not stall the
/// others: the attempts run outside the delivering threads (S4-3, X1c). Packages are fed at
/// a steady rate after the first failed attempt; the healthy targets must keep growing,
/// never standing still for as long as an attempt lasts (2 s, the default kept here on
/// purpose). Before X1c each retry held the thread for those 2 s.
fn blackhole_does_not_stall(test: &str, cell: Cell) {
    let hole = blackhole();
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let real: Vec<String> = (0..cell.targets())
        .map(|i| ctx.key(&format!("t{}", i)))
        .collect();
    let mut clients: Vec<String> = real
        .iter()
        .enumerate()
        .map(|(i, k)| ctx.client_yaml(&format!("t{}", i), k, ""))
        .collect();
    // Last, so a stall cannot hide before the first delivery
    clients.push(ctx.client_yaml_at("hole", "rmtest:hole", hole.port));
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let p = ctx.spawn(&cfg);
    ctx.wait_for("the first connect attempt failed", |_| {
        p.stderr().contains("Error while connecting")
    });
    // 6 s of packages: retries come 1 s and 3 s after the first failure, each lasting 2 s
    let n = 6000;
    let feeder = feed(&ctx, &src, n);
    // The sender holds packages 1 s (ordering): measure once they flow
    let delivered = |c: &mut Ctx| real.iter().map(|k| c.llen(k)).sum::<usize>();
    ctx.wait_for("the first delivery", |c| delivered(c) > 0);
    let (mut last, mut since, mut worst) = (0, Instant::now(), Duration::ZERO);
    while !feeder.is_finished() {
        let now = delivered(&mut ctx);
        if now != last {
            worst = worst.max(since.elapsed());
            last = now;
            since = Instant::now();
        }
        thread::sleep(Duration::from_millis(20));
    }
    feeder.join().unwrap();
    let mut p = p;
    p.stop(&ctx.pid_file());
    assert!(
        worst < Duration::from_millis(1000),
        "the healthy targets stood still for {:?}",
        worst
    );
}

matrix!(target_closes, target_closes_mid_run, 2);
matrix!(blackhole_no_stall, blackhole_does_not_stall, 2);
matrix!(target_blackhole, blackhole_target, 2);
matrix!(target_silent, silent_target, 2);
matrix!(target_unreachable, target_unreachable_from_start, 2);
matrix!(wrongtype, wrongtype_target, 2);
matrix!(stop_target_down, stop_with_a_target_down, 4);
matrix!(after_signal, push_after_the_signal, 2);

/// With ordering, what the stop flushes towards targets that cannot be reached is counted
/// as dropped and reaches the status file in the last write, after the stop; the program
/// still exits 0 (exit codes are stage 4, M3-h). Ordering off holds nothing to flush, so
/// those cells cannot differ from an ordinary stop.
fn dropped_at_the_stop(test: &str, spreader: bool, children: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let n = 300;
    let mut clients = vec![ctx.client_yaml_at("down", "rmtest:down", closed_port())];
    if spreader {
        clients.push(ctx.client_yaml_at("down2", "rmtest:down2", closed_port()));
    }
    // Held for an hour: everything is still in the buffer at the stop
    let extra = "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 3600\n";
    let mode = if spreader { "spreader" } else { "replicant" };
    let cfg = ctx.config(&src, children, mode, extra, &clients);
    let mut p = ctx.spawn(&cfg);
    let input: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &input);
    ctx.wait_for("every package in the buffer", |c| c.llen(&src) == 0);
    p.stop(&ctx.pid_file());
    let json = std::fs::read_to_string(ctx.dir.join("rm.status")).unwrap();
    for (key, want) in [("total_in", n), ("total_out", 0), ("total_drop", n)] {
        assert!(
            json.contains(&format!("\"{}\":{}", key, want)),
            "{} != {} in {}",
            key,
            want,
            json
        );
    }
    assert!(p.stdout().contains("Dropped"), "{}", p.stdout());
}

#[test]
fn dropped_at_the_stop_replicant_one() {
    dropped_at_the_stop("drop_stop_rep_1", false, 1);
}

#[test]
fn dropped_at_the_stop_replicant_several() {
    dropped_at_the_stop("drop_stop_rep_2", false, 2);
}

#[test]
fn dropped_at_the_stop_spreader_one() {
    dropped_at_the_stop("drop_stop_spr_1", true, 1);
}

#[test]
fn dropped_at_the_stop_spreader_several() {
    dropped_at_the_stop("drop_stop_spr_2", true, 2);
}

/// An `ordering_buffer_time` past the clock's range (valid for 1.0.8) holds every package
/// until the stop, which flushes them in order; before, the sender panicked (M3R-13).
fn buffer_time_beyond_the_clock(test: &str, spreader: bool, children: u16) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let mut clients = vec![ctx.client_yaml("t0", &t0, "")];
    if spreader {
        clients.push(ctx.client_yaml("t1", &t1, ""));
    }
    let extra = "ordering: '\"ts\": *(?P<ts>\\d+),'\nordering_buffer_time: 18446744073709551615\n";
    let mode = if spreader { "spreader" } else { "replicant" };
    let cfg = ctx.config(&src, children, mode, extra, &clients);
    let mut p = ctx.spawn(&cfg);
    let n = 6;
    let input: Vec<String> = (0..n).rev().map(seq_pkg).collect();
    ctx.push(&src, &input);
    ctx.wait_for("every package in the buffer", |c| c.llen(&src) == 0);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(ctx.llen(&t0) + ctx.llen(&t1), 0, "released before the stop");
    p.stop(&ctx.pid_file());
    let cell = Cell {
        ordering: true,
        spreader,
        children,
    };
    let lists = if spreader {
        vec![ctx.list(&t0), ctx.list(&t1)]
    } else {
        vec![ctx.list(&t0)]
    };
    assert_all(cell, &lists, n);
}

#[test]
fn buffer_time_beyond_the_clock_replicant_one() {
    buffer_time_beyond_the_clock("hold_max_rep_1", false, 1);
}

#[test]
fn buffer_time_beyond_the_clock_replicant_several() {
    buffer_time_beyond_the_clock("hold_max_rep_2", false, 2);
}

#[test]
fn buffer_time_beyond_the_clock_spreader_one() {
    buffer_time_beyond_the_clock("hold_max_spr_1", true, 1);
}

#[test]
fn buffer_time_beyond_the_clock_spreader_several() {
    buffer_time_beyond_the_clock("hold_max_spr_2", true, 2);
}
