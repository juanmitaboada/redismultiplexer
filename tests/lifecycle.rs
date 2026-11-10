//! Process lifecycle: version, invalid configuration, SIGINT.

mod common;

use common::*;
use std::process::Command;
use std::thread;
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
    // rest stays in the source, in order.
    let mut ctx = Ctx::new("sigint_nonempty");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let all = packages("p", 50000);
    ctx.push(&src, &all);
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("first package delivered", |c| c.llen(&t0) > 0);
    thread::sleep(Duration::from_millis(500));
    assert!(
        ctx.llen(&src) > 0,
        "source drained before SIGINT; raise the package count"
    );
    p.stop(&ctx.pid_file());
    let delivered = ctx.list(&t0);
    let left = ctx.list(&src);
    assert_eq!(delivered.len() + left.len(), all.len());
    assert_eq!(delivered[..], all[..delivered.len()]);
    assert_eq!(left[..], all[delivered.len()..]);
}
