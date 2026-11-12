//! Queue limits: hardlimit / softlimit and deleteblock, over the matrix (ordering on and
//! off, replicant and spreader, one and several children). Every worker and the sender
//! keep their own limits state (L2a, shared state, is M4), so with several threads each
//! one finds a target stuck or full on its own.

mod common;

use common::*;
use std::thread;
use std::time::Duration;

const LIMITS: &str = "    timelimit: 1\n    checklimit: 1\n    softlimit: 5\n    hardlimit: 10\n";

/// Messages `<id> - t0 :: <channel> <word> (Len: n)` printed by the workers (ids below
/// `children`; the sender's id is `children`). Counted by message, not by line: a line
/// is written in several pieces, so threads printing at once can put two messages, or a
/// colour code of another, on one line. The id is the run of digits right before " - t0".
fn worker_lines(stderr: &str, word: &str, children: u16) -> usize {
    let at: Vec<usize> = stderr.match_indices(" - t0 :: ").map(|(i, _)| i).collect();
    at.iter()
        .enumerate()
        .filter(|&(k, &i)| {
            let end = at.get(k + 1).copied().unwrap_or(stderr.len());
            let msg = &stderr[i..end];
            let said = msg.find('!').is_some_and(|b| msg[..=b].ends_with(word));
            let head = &stderr[..i];
            let start = head
                .rfind(|c: char| !c.is_ascii_digit())
                .map_or(0, |d| d + 1);
            said && head[start..].parse::<u16>().is_ok_and(|id| id < children)
        })
        .count()
}

/// A target at its hardlimit takes nothing while it is stuck (replicant: dropped;
/// spreader: the healthy target takes them) and takes packages again once it is under
/// its softlimit
fn stall_and_release(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let mut clients = vec![ctx.client_yaml("t0", &t0, LIMITS)];
    if cell.spreader {
        clients.push(ctx.client_yaml("t1", &t1, ""));
    }
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    let lost: Vec<String> = (0..20).map(seq_pkg).collect();
    ctx.push(&src, &lost);
    if cell.spreader {
        ctx.wait_drained(&src, &[(&t1, 20)]);
    } else {
        ctx.wait_drained(&src, &[(&t0, 10)]);
        // With ordering the sender holds them for its 1 s buffer before it drops them
        thread::sleep(Duration::from_millis(if cell.ordering {
            1500
        } else {
            300
        }));
    }
    assert_eq!(ctx.list(&t0), filler, "stuck target received packages");
    assert!(p.stderr().contains("stuck!"), "no stuck line");

    // Under softlimit; wait until every worker that found it stuck found it freed on
    // its idle path (with ordering the workers hold the packages and the sender checks:
    // no worker line). The sender has no idle path: its next check frees it (timelimit)
    ctx.ltrim(&t0, 8, -1);
    let children = cell.children;
    ctx.wait_for("every stuck worker freed", |_| {
        let err = p.stderr();
        let stuck = worker_lines(&err, "stuck!", children);
        worker_lines(&err, "freed!", children) == stuck
    });
    let fresh: Vec<String> = (100..105).map(seq_pkg).collect();
    ctx.push(&src, &fresh);
    if cell.spreader {
        ctx.wait_for("every fresh package delivered", |c| {
            c.llen(&src) == 0 && c.llen(&t0) + c.llen(&t1) == 2 + 20 + 5
        });
    } else {
        ctx.wait_drained(&src, &[(&t0, 7)]);
    }
    p.stop(&ctx.pid_file());
    let got0 = ctx.list(&t0);
    assert_eq!(&got0[..2], &filler[8..], "the kept packages moved");
    let mut new: Vec<String> = got0[2..].to_vec();
    if cell.spreader {
        let got1 = ctx.list(&t1);
        let mut first: Vec<String> = got1[..20].to_vec();
        if !cell.in_order() {
            first.sort_by_key(|s| ts_in(s.as_bytes()));
        }
        assert_eq!(first, lost, "t1 did not take every package t0 refused");
        new.extend(got1[20..].iter().cloned());
    } else if cell.in_order() {
        assert_eq!(new, fresh, "fresh packages out of order");
    }
    new.sort();
    let mut want = fresh;
    want.sort();
    assert_eq!(new, want, "not every fresh package exactly once");
}

/// A target at its hardlimit with deleteblock: the oldest packages go, in blocks of
/// `deleteblock`, until it is under hardlimit; delivery goes on
fn deleteblock_trims(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let limits = format!("{}    deleteblock: 4\n", LIMITS);
    let mut clients = vec![ctx.client_yaml("t0", &t0, &limits)];
    if cell.spreader {
        clients.push(ctx.client_yaml("t1", &t1, ""));
    }
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    let new = seq_pkg(7);
    ctx.push(&src, std::slice::from_ref(&new));
    ctx.wait_for("the package delivered", |c| {
        c.llen(&src) == 0 && (c.list(&t0).contains(&new) || c.list(&t1).contains(&new))
    });
    p.stop(&ctx.pid_file());
    let got0 = ctx.list(&t0);
    let old: Vec<String> = got0
        .iter()
        .filter(|s| s.starts_with("old"))
        .cloned()
        .collect();
    // Threads that read the length before another trimmed may each trim a block (L2a),
    // down to an empty list
    let trimmed = filler.len() - old.len();
    assert!(
        trimmed >= 4 && (trimmed % 4 == 0 || trimmed == filler.len()) && old == filler[trimmed..],
        "not the oldest whole blocks trimmed: kept {:?}",
        old
    );
    let mut count = got0.iter().filter(|s| **s == new).count();
    if cell.spreader {
        count += ctx.list(&t1).iter().filter(|s| **s == new).count();
    }
    assert_eq!(count, 1, "the package not delivered exactly once");
}

matrix!(stall, stall_and_release, 4);
matrix!(deleteblock, deleteblock_trims, 4);

/// Every limit at 0 is a target without limits (L4c-1, M3F-17): before, the start
/// accepted it as such but the delivery read `hardlimit: 0` as a limit, so the target
/// was stuck at length 0 and every package dropped. One cell: the keys are read once at
/// start, the same for every thread.
#[test]
fn zero_limits_are_no_limits() {
    let mut ctx = Ctx::new("zero_limits");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let zeros = "    timelimit: 0\n    checklimit: 0\n    softlimit: 0\n    hardlimit: 0\n";
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, zeros)],
    );
    let mut p = ctx.spawn(&cfg);
    let all = packages("p", 20);
    ctx.push(&src, &all);
    ctx.wait_drained(&src, &[(&t0, 20)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), all);
    assert!(
        !p.stderr().contains("stuck!"),
        "a target without limits got stuck"
    );
}

/// `deleted` counts what deleteblock removed (L4c-4): a list of 5 over a hardlimit of 3,
/// with `deleteblock: 100`, loses 5 elements and counts 5 (before: 100). One thread (one
/// child, ordering off): threads that read the length before another trimmed each count
/// their block (L2a, M4), so only one thread gives an exact figure.
#[test]
fn deleteblock_counts_what_it_removed() {
    let mut ctx = Ctx::new("deleted_count");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&t0, &packages("old", 5));
    let limits = "    timelimit: 1\n    checklimit: 1\n    softlimit: 2\n    hardlimit: 3\n    deleteblock: 100\n";
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, limits)],
    );
    let mut p = ctx.spawn(&cfg);
    // The check comes with an offer (the idle path checks stuck targets only, M3F-7)
    ctx.push(&src, &["new".to_string()]);
    ctx.wait_for("the list trimmed and the package delivered", |c| {
        c.llen(&src) == 0 && c.list(&t0) == ["new"]
    });
    // The periodic write after 10 s, then the last one after the stop (as delivery.rs)
    let status = ctx.dir.join("rm.status");
    let mut json = String::new();
    ctx.wait_for("a complete status file", |_| {
        json = std::fs::read_to_string(&status).unwrap_or_default();
        json.trim_end().ends_with('}') && json.contains("\"total_deleted\":")
    });
    let first = total_deleted(&json);
    p.stop(&ctx.pid_file());
    let last = total_deleted(&std::fs::read_to_string(&status).unwrap());
    assert_eq!(first + last, 5, "deleted: {} + {}", first, last);
}

fn total_deleted(json: &str) -> u64 {
    let key = "\"total_deleted\":";
    let at = json.find(key).expect("total_deleted") + key.len();
    json[at..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// A target without limits is never sent LLEN, while packages flow or while the source
/// is idle (M3F-7: 1.0.8 and the parent sent one a second per target and thread, on
/// both paths, and nothing read the answer)
fn no_llen_without_limits(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let targets: Vec<SlowTarget> = (0..cell.targets())
        .map(|_| SlowTarget::start(Duration::ZERO))
        .collect();
    let clients: Vec<String> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            ctx.client_yaml_at(&format!("t{}", i), &format!("rmtest:{}:{}", tag, i), t.port)
        })
        .collect();
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    // Packages for 2.5 s, then 2.5 s idle (ordering: the sender's last batch after 1 s)
    for start in (0..500u64).step_by(50) {
        let batch: Vec<String> = (start..start + 50).map(seq_pkg).collect();
        ctx.push(&src, &batch);
        thread::sleep(Duration::from_millis(250));
    }
    let total = |t: &[SlowTarget]| -> usize { t.iter().map(|t| t.count()).sum() };
    let want = if cell.spreader {
        500
    } else {
        500 * targets.len()
    };
    ctx.wait_for("every package delivered", |_| total(&targets) == want);
    thread::sleep(Duration::from_millis(2500));
    p.stop(&ctx.pid_file());
    let llens: Vec<usize> = targets.iter().map(|t| t.llens()).collect();
    assert!(
        llens.iter().all(|&n| n == 0),
        "LLEN sent to a target without limits: {:?}",
        llens
    );
}

/// A stuck target is re-checked once a second while the source is idle, whatever its
/// timelimit and checklimit say (L4c-3): here they would wait an hour or 1000 packages,
/// and the parent's idle path ran the timelimit and countdown check, so it stayed stuck.
/// Workers only: the sender has no idle path, its next check frees it.
fn stuck_freed_while_idle(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    ctx.push(&t0, &packages("old", 10));
    let limits = "    timelimit: 3600\n    checklimit: 1000\n    softlimit: 5\n    hardlimit: 10\n";
    let mut clients = vec![ctx.client_yaml("t0", &t0, limits)];
    if cell.spreader {
        clients.push(ctx.client_yaml("t1", &t1, ""));
    }
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    let children = cell.children;
    // Packages without a ts: the workers deliver them themselves, also with ordering, and
    // check the target at their first offer. Only workers that got one find it stuck.
    ctx.push(&src, &packages("p", 40));
    ctx.wait_for("the source drained and a worker found it stuck", |c| {
        c.llen(&src) == 0 && worker_lines(&p.stderr(), "stuck!", children) > 0
    });
    thread::sleep(Duration::from_millis(300));
    let stuck = worker_lines(&p.stderr(), "stuck!", children);
    ctx.ltrim(&t0, 8, -1);
    let start = std::time::Instant::now();
    ctx.wait_for("every stuck worker freed it", |_| {
        worker_lines(&p.stderr(), "freed!", children) == stuck
    });
    let took = start.elapsed();
    p.stop(&ctx.pid_file());
    assert!(took < Duration::from_secs(5), "freed after {:?}", took);
}

matrix!(no_llen, no_llen_without_limits, 4);
matrix!(idle_recheck, stuck_freed_while_idle, 4);
