//! How the program connects (M3-i): a structured description instead of a URL, so a
//! database other than 0, a Redis 6 user and passwords with URL characters work (M1F-7,
//! M1F-10), and TLS when a TLS server is given.

mod common;

use common::*;
use std::time::Duration;

/// A key of database `db` on the test server, deleted when dropped
struct OtherDb {
    con: redis::Connection,
    keys: Vec<String>,
}

impl OtherDb {
    fn open(ctx: &Ctx, db: i64) -> OtherDb {
        let mut con = redis::Client::open(ctx.ep.url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        let _: () = redis::cmd("SELECT").arg(db).query(&mut con).unwrap();
        OtherDb {
            con,
            keys: Vec::new(),
        }
    }

    fn key(&mut self, ctx: &Ctx, name: &str) -> String {
        let k = format!("{}:{}", ctx.prefix, name);
        self.keys.push(k.clone());
        k
    }

    fn llen(&mut self, key: &str) -> usize {
        redis::cmd("LLEN").arg(key).query(&mut self.con).unwrap()
    }

    fn list(&mut self, key: &str) -> Vec<String> {
        redis::cmd("LRANGE")
            .arg(key)
            .arg(0)
            .arg(-1)
            .query(&mut self.con)
            .unwrap()
    }
}

impl Drop for OtherDb {
    fn drop(&mut self) {
        if !self.keys.is_empty() {
            let _: Result<i64, _> = redis::cmd("DEL").arg(&self.keys).query(&mut self.con);
        }
    }
}

/// M1F-10: source and target in database 1; 1.0.8 always used database 0. With ordering
/// the sender delivers, through its own connections: both cells, so a sender that built
/// its description apart (database 0) fails the second.
fn database_selection(test: &str, ordering: bool) {
    let mut ctx = Ctx::new(test);
    if ctx.ep.database != 0 {
        return; // the test URL selects a database already: the configuration sets it
    }
    let mut db1 = OtherDb::open(&ctx, 1);
    let src = db1.key(&ctx, "src");
    let t0 = db1.key(&ctx, "t0");
    // seq_pkg: with ordering they carry a timestamp and go through the sender
    let all: Vec<String> = (0..20).map(seq_pkg).collect();
    let _: i64 = redis::cmd("RPUSH")
        .arg(&src)
        .arg(&all)
        .query(&mut db1.con)
        .unwrap();
    let client = ctx.client_yaml("t0", &t0, "    database: 1\n");
    let extra = format!("database: 1\n{}", if ordering { ORDERING_1S } else { "" });
    let cfg = ctx.config(&src, 2, "replicant", &extra, &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("the packages in database 1", |_| {
        db1.llen(&src) == 0 && db1.llen(&t0) == all.len()
    });
    p.stop(&ctx.pid_file());
    let mut got = db1.list(&t0);
    got.sort();
    let mut want = all;
    want.sort();
    assert_eq!(got, want);
    assert_eq!(ctx.llen(&t0), 0, "a package went to database 0");
}

#[test]
fn database_selection_workers() {
    database_selection("database_off", false);
}

#[test]
fn database_selection_sender() {
    database_selection("database_on", true);
}

/// An ACL user created for one test and deleted with it
struct AclUser {
    con: redis::Connection,
    name: String,
}

impl Drop for AclUser {
    fn drop(&mut self) {
        let _: Result<i64, _> = redis::cmd("ACL")
            .arg("DELUSER")
            .arg(&self.name)
            .query(&mut self.con);
    }
}

/// Each line of `text` indented by four spaces (client lines of the YAML)
fn indented(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// M1F-7 and M3-i: a Redis 6 user whose password holds every character the URL of 1.0.8
/// broke on ('#', '/', '?' panicked, '%41' was read as 'A', a tab did not connect). Both
/// cells of ordering, as database_selection.
fn username_and_special_password(test: &str, ordering: bool) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let password = "p@ss:w/rd#?%41\tx y";
    let user = format!("rmtest_{}_{}", std::process::id(), test);
    let mut con = redis::Client::open(ctx.ep.url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    let _: () = redis::cmd("ACL")
        .arg(&[
            "SETUSER",
            &user,
            "on",
            &format!(">{}", password),
            "~rmtest:*",
            "+@all",
        ])
        .query(&mut con)
        .unwrap();
    let _guard = AclUser {
        con,
        name: user.clone(),
    };
    let all: Vec<String> = (0..20).map(seq_pkg).collect();
    ctx.push(&src, &all);
    let access = format!("username: {}\npassword: {}\n", q(&user), q(password));
    let client = format!(
        "  - name: 't0'\n    ssl: {}\n    hostname: {}\n    port: {}\n{}    channel: {}\n",
        ctx.ep.ssl,
        q(&ctx.ep.host),
        ctx.ep.port,
        indented(&access),
        q(&t0)
    );
    // The root block's own `password:` line is the test URL's: replaced by the user's
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(&src, 2, "replicant", extra, &[client]);
    let yaml = std::fs::read_to_string(&cfg).unwrap();
    let root_password = format!("password: {}\n", q(&ctx.ep.password));
    let yaml = yaml.replacen(&root_password, &access, 1);
    std::fs::write(&cfg, yaml).unwrap();
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, all.len())]);
    p.stop(&ctx.pid_file());
    let mut got = ctx.list(&t0);
    got.sort();
    let mut want = all;
    want.sort();
    assert_eq!(got, want);
    assert!(!p.stderr().contains("Couldn't connect"), "{}", p.stderr());
}

#[test]
fn username_and_special_password_workers() {
    username_and_special_password("acl_off", false);
}

#[test]
fn username_and_special_password_sender() {
    username_and_special_password("acl_on", true);
}

/// The TLS Redis of the environment, if given: RM_TEST_REDIS_TLS_URL (rediss://host:port,
/// no password) and RM_TEST_REDIS_TLS_CA (the CA file): (host, port, CA)
fn tls_redis(test: &str) -> Option<(String, String, String)> {
    let (Ok(url), Ok(ca)) = (
        std::env::var("RM_TEST_REDIS_TLS_URL"),
        std::env::var("RM_TEST_REDIS_TLS_CA"),
    ) else {
        eprintln!(
            "{}: skipped, RM_TEST_REDIS_TLS_URL or RM_TEST_REDIS_TLS_CA not set",
            test
        );
        return None;
    };
    let hostport = url
        .strip_prefix("rediss://")
        .expect("RM_TEST_REDIS_TLS_URL starts with rediss://");
    let (host, port) = hostport.trim_end_matches('/').rsplit_once(':').unwrap();
    Some((host.to_string(), port.to_string(), ca))
}

/// redis-cli against the TLS Redis
fn tls_cli(host: &str, port: &str, ca: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("redis-cli")
        .args(["--tls", "--cacert", ca, "-h", host, "-p", port])
        .args(args)
        .output()
        .expect("redis-cli for the TLS Redis");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn tls_target() {
    tls_target_cell("tls_target", false);
}

#[test]
fn tls_target_sender() {
    // M3R-25: with ordering the sender delivers, through its own connections
    tls_target_cell("tls_target_sender", true);
}

/// M3R-4 (PKG): a target over TLS, the server's certificate signed by a CA the program is
/// told to trust. Runs only where a TLS Redis is given.
fn tls_target_cell(test: &str, ordering: bool) {
    let Some((host, port, ca)) = tls_redis(test) else {
        return;
    };
    let (host, port) = (host.as_str(), port.as_str());
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let all: Vec<String> = (0..20).map(seq_pkg).collect();
    ctx.push(&src, &all);
    let client = format!(
        "  - name: 't0'\n    ssl: true\n    hostname: {}\n    port: {}\n    password: ''\n    channel: {}\n",
        q(host),
        port,
        q(&t0)
    );
    let extra = if ordering { ORDERING_1S } else { "" };
    let cfg = ctx.config(&src, 2, "replicant", extra, &[client]);
    let mut p = ctx.spawn_env(&cfg, "SSL_CERT_FILE", &ca);
    ctx.wait_for("the source drained", |c| c.llen(&src) == 0);
    // Read back over TLS with the same CA (with ordering after the buffer time)
    let start = std::time::Instant::now();
    let mut got: Vec<String> = Vec::new();
    while start.elapsed() < Duration::from_secs(10) {
        got = tls_cli(host, port, &ca, &["LRANGE", &t0, "0", "-1"])
            .lines()
            .map(|l| l.to_string())
            .collect();
        if got.len() >= all.len() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    p.stop(&ctx.pid_file());
    tls_cli(host, port, &ca, &["DEL", &t0]);
    got.sort();
    let mut want = all;
    want.sort();
    assert_eq!(got, want, "{}", p.stderr());
}

#[test]
fn tls_source() {
    // M3R-25: a source over TLS (the TLS Redis), a plain target
    let Some((host, port, ca)) = tls_redis("tls_source") else {
        return;
    };
    let mut ctx = Ctx::new("tls_source");
    let src = format!("rmtest:tls_source:{}:src", std::process::id());
    let t0 = ctx.key("t0");
    let all: Vec<String> = (0..20).map(seq_pkg).collect();
    let mut push: Vec<&str> = vec!["RPUSH", &src];
    push.extend(all.iter().map(|p| p.as_str()));
    tls_cli(&host, &port, &ca, &push);
    let cfg = ctx.config(&src, 2, "replicant", "", &[ctx.client_yaml("t0", &t0, "")]);
    let yaml = std::fs::read_to_string(&cfg).unwrap();
    let root = format!(
        "ssl: {}\nhostname: {}\nport: {}\npassword: {}\n",
        ctx.ep.ssl,
        q(&ctx.ep.host),
        ctx.ep.port,
        q(&ctx.ep.password)
    );
    assert!(yaml.contains(&root), "{}", yaml);
    let tls_root = format!(
        "ssl: true\nhostname: {}\nport: {}\npassword: ''\n",
        q(&host),
        port
    );
    std::fs::write(&cfg, yaml.replacen(&root, &tls_root, 1)).unwrap();
    let mut p = ctx.spawn_env(&cfg, "SSL_CERT_FILE", &ca);
    ctx.wait_for("every package at the target", |c| c.llen(&t0) == all.len());
    p.stop(&ctx.pid_file());
    let left = tls_cli(&host, &port, &ca, &["LLEN", &src]);
    tls_cli(&host, &port, &ca, &["DEL", &src]);
    assert_eq!(left.trim(), "0");
    let mut got = ctx.list(&t0);
    got.sort();
    let mut want = all;
    want.sort();
    assert_eq!(got, want, "{}", p.stderr());
}

#[test]
fn connections_are_named() {
    // CLIENT SETNAME on every connection: source, each worker's targets, the sender's
    let mut ctx = Ctx::new("names");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let client = ctx.client_yaml("t 0", &t0, "");
    let cfg = ctx.config(&src, 2, "replicant", ORDERING_1S, &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.push(&src, &[seq_pkg(1)]);
    ctx.wait_drained(&src, &[(&t0, 1)]);
    let list: String = ctx.cmd(&["CLIENT", "LIST"]);
    p.stop(&ctx.pid_file());
    for name in [
        "redismultiplexer:source:-:src0",
        "redismultiplexer:source:-:src1",
        "redismultiplexer:source:t_0:w0",
        "redismultiplexer:source:t_0:w1",
        "redismultiplexer:source:t_0:sender",
    ] {
        assert!(
            list.contains(&format!("name={} ", name)),
            "{} not in\n{}",
            name,
            list
        );
    }
}

/// A target that refuses CLIENT SETNAME, or closes the connection on it, still gets
/// every package: the name is never a condition for a connection (M3R-2's proxy case)
fn setname_is_optional(test: &str, on_client: OnClient) {
    let mut ctx = Ctx::new(test);
    let src = ctx.key("src");
    let target = SlowTarget::start_with(std::time::Duration::ZERO, on_client);
    let all = packages("n", 30);
    ctx.push(&src, &all);
    let client = ctx.client_yaml_at("t0", "rmtest:named", target.port);
    let cfg = ctx.config(&src, 2, "replicant", "", &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("every package at the target", |c| {
        c.llen(&src) == 0 && target.count() == all.len()
    });
    p.stop(&ctx.pid_file());
    let mut got = target.pushed();
    got.sort();
    assert_eq!(got, all);
    assert!(
        !p.stderr().contains("Error while connecting"),
        "{}",
        p.stderr()
    );
}

#[test]
fn setname_refused_is_ignored() {
    setname_is_optional("setname_refused", OnClient::Error);
}

#[test]
fn setname_closing_peer_gets_a_connection_without_it() {
    setname_is_optional("setname_closed", OnClient::Close);
}

#[test]
fn host_in_upper_case_connects() {
    // The host goes to the resolver as written: a name in upper case resolves as the lower
    // case one does (the differential run pins that the description keeps the case)
    let mut ctx = Ctx::new("upper_host");
    if !["127.0.0.1", "localhost"].contains(&ctx.ep.host.as_str()) || ctx.ep.ssl {
        return; // the test Redis is not local: no name in upper case to give it
    }
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let all = packages("up", 10);
    ctx.push(&src, &all);
    let client = ctx.client_yaml("t0", &t0, "").replacen(
        &format!("hostname: {}", q(&ctx.ep.host)),
        "hostname: 'LOCALHOST'",
        1,
    );
    assert!(client.contains("LOCALHOST"), "{}", client);
    let cfg = ctx.config(&src, 1, "replicant", "", &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, all.len())]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), all);
}

#[test]
fn ipv6_target_with_and_without_brackets() {
    // M3R-28: `::1` and `[::1]` both connect. Runs only where a Redis listens on the IPv6
    // loopback without a password: RM_TEST_REDIS_IPV6_PORT (its port)
    let Ok(port) = std::env::var("RM_TEST_REDIS_IPV6_PORT") else {
        eprintln!(
            "ipv6_target_with_and_without_brackets: skipped, RM_TEST_REDIS_IPV6_PORT not set"
        );
        return;
    };
    let mut ctx = Ctx::new("ipv6");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let t1 = ctx.key("t1");
    let all = packages("v6", 10);
    ctx.push(&src, &all);
    let client = |name: &str, host: &str, key: &str| {
        format!(
            "  - name: '{}'\n    ssl: false\n    hostname: '{}'\n    port: {}\n    password: ''\n    channel: {}\n",
            name,
            host,
            port,
            q(key)
        )
    };
    let clients = [
        client("plain", "::1", &t0),
        client("bracketed", "[::1]", &t1),
    ];
    let cfg = ctx.config(&src, 1, "replicant", "", &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("the source drained", |c| c.llen(&src) == 0);
    p.stop(&ctx.pid_file());
    for key in [&t0, &t1] {
        let out = std::process::Command::new("redis-cli")
            .args(["-h", "::1", "-p", &port, "LRANGE", key, "0", "-1"])
            .output()
            .expect("redis-cli to read the IPv6 target");
        let _ = std::process::Command::new("redis-cli")
            .args(["-h", "::1", "-p", &port, "DEL", key])
            .output();
        let got: Vec<String> = String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(got, all, "{}: {}", key, p.stderr());
    }
}
