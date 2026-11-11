//! Process lifecycle: version, invalid configuration, SIGINT.

mod common;

use common::*;
use std::process::Command;
use std::time::Duration;

#[test]
fn version_flag() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("RedisMultiplexer v{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_banner_shows_the_version_only() {
    // The build date was dropped with the build-time crate (reproducible builds).
    let out = Command::new(bin()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let banner = format!("RedisMultiplexer v{}\n", env!("CARGO_PKG_VERSION"));
    assert!(stdout.contains(&banner), "{:?}", stdout);
}

/// The line of `out` that holds the version banner; it must carry no build date.
fn banner_line(out: &str) -> String {
    let banner = format!("RedisMultiplexer v{}", env!("CARGO_PKG_VERSION"));
    let line = out
        .lines()
        .find(|l| l.contains(&banner))
        .unwrap_or_else(|| panic!("no banner in {:?}", out))
        .to_string();
    let rest = &line[line.find(&banner).unwrap() + banner.len()..];
    assert!(!rest.contains('('), "build date in {:?}", line);
    line
}

#[test]
fn start_banner_shows_the_version_only() {
    let mut ctx = Ctx::new("start_banner");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&src, &packages("b", 1));
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    p.stop(&ctx.pid_file());
    banner_line(&p.stdout());
}

#[test]
fn no_argument_banner_shows_the_version_only() {
    let out = Command::new(bin()).output().unwrap();
    banner_line(&String::from_utf8_lossy(&out.stdout));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Usage"),
        "{:?}",
        out
    );
}

#[test]
fn invalid_config_exits_zero_current_behaviour() {
    // M1F-3: a configuration error is printed but the exit status is 0.
    let mut ctx = Ctx::new("invalid_config");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(&src, 1, "foo", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    let st = p.wait_exit();
    assert!(st.success(), "exit status: {:?}", st);
    assert!(
        p.stderr().contains("Mode 'foo' is unknown"),
        "{}",
        p.stderr()
    );
    assert!(!ctx.pid_file().exists());
}

#[test]
fn sigint_with_nonempty_source() {
    // Documents what a clean stop loses with packages still queued: nothing.
    // Without ordering every popped package is delivered before exit and the
    // rest stays in the source, in order; with one child also delivered in order.
    let run = stop_under_load("sigint_nonempty", 1, false, Duration::from_millis(500));
    assert!(run
        .delivered
        .iter()
        .copied()
        .eq(0..run.delivered.len() as u64));
    assert!(!run.left.is_empty());
}
