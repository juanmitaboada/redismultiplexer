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
fn bad_client_regex_is_refused_at_start() {
    // M1F-8: the client regex was compiled with unwrap() in each worker, which panicked
    // them all after the program had started; it is now a configuration error.
    let mut ctx = Ctx::new("bad_client_regex");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&src, &packages("x", 3));
    let client = ctx.client_yaml("t0", &t0, "    filter: '('\n");
    let cfg = ctx.config(&src, 1, "replicant", "", &[client]);
    let mut p = ctx.spawn(&cfg);
    p.wait_exit();
    assert!(
        p.stderr()
            .contains("Client 't0' has a filter Regex '(' that doesn't compile"),
        "{}",
        p.stderr()
    );
    assert!(!p.stderr().contains("panicked"), "{}", p.stderr());
    assert_eq!(ctx.llen(&src), 3);
    assert!(!ctx.pid_file().exists());
}

#[test]
fn invalid_config_exits_two() {
    // M3-h = ha (M1F-3): a configuration error exits 2 (1.0.8: 0)
    let mut ctx = Ctx::new("invalid_config");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let cfg = ctx.config(&src, 1, "foo", "", &[ctx.client_yaml("t0", &t0, "")]);
    let mut p = ctx.spawn(&cfg);
    let st = p.wait_exit();
    assert_eq!(st.code(), Some(2), "exit status: {:?}", st);
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

#[test]
fn usage_errors_exit_two() {
    // No argument, a file that is not there, a file that is not YAML: 2 (M3-h = ha)
    let out = Command::new(bin()).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{:?}", out);
    let out = Command::new(bin())
        .arg("/nonexistent/redismultiplexer.yaml")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{:?}", out);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not found"),
        "{:?}",
        out
    );
    let dir = std::env::temp_dir().join(format!("rmtest-usage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.yaml");
    std::fs::write(&bad, "name: [unclosed\n").unwrap();
    let out = Command::new(bin()).arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{:?}", out);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unwritable_pid_file_exits_two() {
    let mut ctx = Ctx::new("pid_unwritable");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    ctx.push(&src, &packages("p", 2));
    let cfg = ctx.config(&src, 1, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    // The pid line of the configuration points into a directory that does not exist
    let text = std::fs::read_to_string(&cfg).unwrap();
    let pid = ctx.pid_file().to_string_lossy().to_string();
    let text = text.replace(&pid, "/nonexistent/rmtest.pid");
    assert_ne!(
        text,
        std::fs::read_to_string(&cfg).unwrap(),
        "no pid line to replace"
    );
    std::fs::write(&cfg, text).unwrap();
    let mut p = ctx.spawn(&cfg);
    let st = p.wait_exit();
    assert_eq!(st.code(), Some(2), "exit status: {:?}", st);
    assert!(p.stderr().contains("Unable to write PID"), "{}", p.stderr());
    assert_eq!(ctx.llen(&src), 2, "it popped packages");
}

#[test]
fn odd_settings_warn_and_start() {
    // M3-m = ma: a typo in a key, an empty client name, two clients with one name, ordering
    // keys without ordering, softlimit above hardlimit: one warning each, and it delivers
    let mut ctx = Ctx::new("odd_settings");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    ctx.push(&src, &packages("p", 3));
    let c0 = ctx.client_yaml(
        "",
        &t0,
        "    filter_untill: '#'\n    softlimit: 50\n    hardlimit: 40\n",
    );
    let c1 = ctx.client_yaml("twin", &t1, "");
    let t2 = ctx.key("t2");
    let c2 = ctx.client_yaml("twin", &t2, "");
    let extra = "ordering_limit: 64\ncolour: blue\n";
    let cfg = ctx.config(&src, 1, "replicant", extra, &[c0, c1, c2]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 3), (&t1, 3)]);
    p.stop(&ctx.pid_file());
    let err = p.stderr();
    for w in [
        "Warning: Unknown key 'filter_untill' in client '': ignored",
        "Warning: Unknown key 'colour' at the root of the configuration: ignored",
        "Warning: A client has an empty name",
        "Warning: Client 'twin' is the name of more than one client",
        "sets ordering_limit without ordering: ignored",
        "has softlimit 50 at or above hardlimit 40",
    ] {
        assert!(err.contains(w), "no {:?} in {}", w, err);
    }
}
