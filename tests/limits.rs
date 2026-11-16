//! Queue limits: hardlimit / softlimit and deleteblock, over the matrix (ordering on and
//! off, replicant and spreader, one and several children). Every worker and the sender
//! keep their own limits state (L2a, shared state, is M4), so with several threads each
//! one finds a target stuck or full on its own.

mod common;

use common::*;
use std::thread;
use std::time::Duration;

const LIMITS: &str = "    timelimit: 1\n    checklimit: 1\n    softlimit: 5\n    hardlimit: 10\n";

/// Lines `<date> RedisMultiplexer: <id> - t0 :: <channel> <word> (Len: n)` printed by the
/// workers (ids below `children`; the sender's id is `children`). Counted by line: a line
/// is written whole in one call (M3-j), so each holds one message; asserted here.
fn worker_lines(stderr: &str, word: &str, children: u16) -> usize {
    stderr
        .lines()
        .filter(|line| {
            let Some(i) = line.find(" - t0 :: ") else {
                return false;
            };
            assert_eq!(
                line.matches(" - t0 :: ").count(),
                1,
                "two messages on one line: {}",
                line
            );
            let said = line[i..]
                .find('!')
                .is_some_and(|b| line[i..i + b + 1].ends_with(word));
            let head = &line[..i];
            let start = head
                .rfind(|c: char| !c.is_ascii_digit())
                .map_or(0, |d| d + 1);
            said && head[start..].parse::<u16>().is_ok_and(|id| id < children)
        })
        .count()
}

/// Messages of the sender (its id is `children`), counted as worker_lines counts
fn sender_lines(stderr: &str, word: &str, children: u16) -> usize {
    worker_lines(stderr, word, children + 1) - worker_lines(stderr, word, children)
}

/// A target at its hardlimit takes nothing while it is stuck (replicant: dropped;
/// spreader: the healthy target takes them) and takes packages again once it is under
/// its softlimit
fn stall_and_release(tag: &str, cell: Cell) {
    stall_and_release_with(tag, cell, LIMITS)
}

/// `deleteblock: 0` is read as not set (C4c-1 = a): the target is stuck at hardlimit and
/// freed under softlimit, as without the key (1.0.8 looped for ever on LTRIM; 4c refused it)
fn deleteblock_zero_stalls(tag: &str, cell: Cell) {
    let limits = format!("{}    deleteblock: 0\n", LIMITS);
    stall_and_release_with(tag, cell, &limits)
}

fn stall_and_release_with(tag: &str, cell: Cell, limits: &str) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let mut clients = vec![ctx.client_yaml("t0", &t0, limits)];
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
    // no worker line; the sender's idle turn or its next check frees it)
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
matrix!(deleteblock_zero, deleteblock_zero_stalls, 4);

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
/// With ordering the sender too: its idle turn re-checks the target (M3R-31).
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
    if cell.ordering {
        // Packages with a ts go through the sender, which finds it stuck too
        ctx.push(&src, &(0..5).map(seq_pkg).collect::<Vec<String>>());
        ctx.wait_for("the sender found it stuck", |_| {
            sender_lines(&p.stderr(), "stuck!", children) > 0
        });
    }
    thread::sleep(Duration::from_millis(300));
    let stuck = worker_lines(&p.stderr(), "stuck!", children);
    ctx.ltrim(&t0, 8, -1);
    let start = std::time::Instant::now();
    ctx.wait_for("every stuck worker freed it", |_| {
        worker_lines(&p.stderr(), "freed!", children) == stuck
            && (!cell.ordering || sender_lines(&p.stderr(), "freed!", children) > 0)
    });
    let took = start.elapsed();
    p.stop(&ctx.pid_file());
    assert!(took < Duration::from_secs(5), "freed after {:?}", took);
}

matrix!(no_llen, no_llen_without_limits, 4);

/// One check trims until the target is under hardlimit, however many blocks that takes
/// (M3R-31): 30 elements, hardlimit 10, deleteblock 4: six trims at the first package
fn deleteblock_trims_in_one_check(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&t0, &packages("old", 30));
    let limits = "    timelimit: 3600\n    checklimit: 1000\n    softlimit: 5\n    hardlimit: 10\n    deleteblock: 4\n";
    let clients = [ctx.client_yaml("t0", &t0, limits)];
    let cfg = ctx.config(&src, cell.children, cell.mode(), cell.extra(), &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_for("the package delivered", |c| {
        c.llen(&src) == 0 && c.list(&t0).last() == Some(&seq_pkg(1))
    });
    p.stop(&ctx.pid_file());
    let got = ctx.list(&t0);
    assert_eq!(got.len(), 7, "{:?}", got);
    assert_eq!(
        &got[..6],
        &packages("old", 30)[24..],
        "not the oldest trimmed"
    );
}

matrix!(deleteblock_loop, deleteblock_trims_in_one_check, 2);

/// Lengths as 64 bits and LTRIM to the end of the list (M3F-5, M3R-31), with a stand-in:
/// a target whose LLEN says 2^32 + 5 (read as 32 bits that is 5, under hardlimit, and no
/// trim) is trimmed with `LTRIM <key> 4 -1` (1.0.8 kept at most 100000000 after the block)
fn long_lists(tag: &str, cell: Cell) {
    let mut ctx = Ctx::new(tag);
    let src = ctx.key("src");
    let target = SlowTarget::scripted_llen(&[(1u64 << 32) + 5, 3]);
    let key = format!("rmtest:{}:t0", tag);
    let limits = "    timelimit: 3600\n    checklimit: 1000\n    softlimit: 5\n    hardlimit: 10\n    deleteblock: 4\n";
    let client = format!("{}{}", ctx.client_yaml_at("t0", &key, target.port), limits);
    let cfg = ctx.config(&src, 1, cell.mode(), cell.extra(), &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_for("the package delivered", |_| target.count() == 1);
    p.stop(&ctx.pid_file());
    assert_eq!(
        target.ltrims(),
        vec![(key, "4".to_string(), "-1".to_string())],
        "{}",
        p.stderr()
    );
}

matrix!(long_list, long_lists, 1);
matrix!(idle_recheck, stuck_freed_while_idle, 4);

/// A hardlimit alone turns the limits on (M3-f = fa): stuck at 10, freed under the default
/// softlimit of 8 (L4c-2), checked at the first offer and then every second or 10000
/// packages (L4c-5). The parent refused the file ("must set all limits"). One cell: the
/// defaults are set once at start, the same for every thread.
#[test]
fn hardlimit_alone() {
    let mut ctx = Ctx::new("hardlimit_alone");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let filler = packages("old", 10);
    ctx.push(&t0, &filler);
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        "",
        &[ctx.client_yaml("t0", &t0, "    hardlimit: 10\n")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &packages("lost", 5));
    ctx.wait_for("stuck", |c| {
        c.llen(&src) == 0 && p.stderr().contains("stuck!")
    });
    // At 8 (the default softlimit) it stays stuck; at 7 it is freed
    ctx.ltrim(&t0, 2, -1);
    thread::sleep(Duration::from_millis(2500));
    assert!(
        !p.stderr().contains("freed!"),
        "freed at the softlimit, not under it"
    );
    ctx.ltrim(&t0, 1, -1);
    ctx.wait_for("freed", |_| p.stderr().contains("freed!"));
    let fresh = packages("new", 3);
    ctx.push(&src, &fresh);
    ctx.wait_drained(&src, &[(&t0, 10)]);
    p.stop(&ctx.pid_file());
    let mut want = filler[3..].to_vec();
    want.extend(fresh);
    assert_eq!(ctx.list(&t0), want);
}

/// P4 = p4a through the binary (M3R-45): with ordering the sender pipelines a chunk, and a
/// limits check that is due first writes and reads what is pending for that target, so
/// each LLEN comes after every RPUSH planned before it. checklimit 1 checks every second
/// package (the counter as in 1.0.8): the k-th LLEN (from 0) after at least 2k packages.
/// One child, replicant: one sender, one target.
#[test]
fn ordering_limits_check_after_the_pending_packages() {
    let mut ctx = Ctx::new("ordering_p4");
    let src = ctx.key("src");
    let target = SlowTarget::start(Duration::ZERO);
    let key = ctx.key("t0");
    let limits = "    timelimit: 3600\n    checklimit: 1\n    softlimit: 50\n    hardlimit: 100\n";
    let client = format!("{}{}", ctx.client_yaml_at("t0", &key, target.port), limits);
    let cfg = ctx.config(&src, 1, "replicant", ORDERING_1S, &[client]);
    let mut p = ctx.spawn(&cfg);
    let n = 300;
    let batch: Vec<String> = (0..n).map(seq_pkg).collect();
    ctx.push(&src, &batch);
    ctx.wait_for("every package delivered", |_| target.count() == n as usize);
    p.stop(&ctx.pid_file());
    let seq = target.commands();
    let mut pushed = 0;
    let mut llens = 0;
    for c in &seq {
        if *c == "LLEN" {
            assert!(
                pushed >= 2 * llens,
                "LLEN {llens} after only {pushed} RPUSH: {}",
                p.stderr()
            );
            llens += 1;
        } else {
            pushed += 1;
        }
    }
    assert!(llens >= n as usize / 2, "{llens} LLEN for {n} packages");
}
