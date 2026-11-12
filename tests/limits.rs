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
    // its idle path. The sender has no idle path: its next check frees it (timelimit)
    ctx.ltrim(&t0, 8, -1);
    let children = cell.children;
    ctx.wait_for("every stuck worker freed", |_| {
        let err = p.stderr();
        let stuck = worker_lines(&err, "stuck!", children);
        stuck > 0 && worker_lines(&err, "freed!", children) == stuck
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
