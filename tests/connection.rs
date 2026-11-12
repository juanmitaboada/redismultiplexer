//! How the program connects (M3-i): a structured description instead of a URL, so a
//! database other than 0, a Redis 6 user and passwords with URL characters work (M1F-7,
//! M1F-10), and TLS when a TLS server is given.

mod common;

use common::*;

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

#[test]
fn database_selection() {
    // M1F-10: source and target in database 1; 1.0.8 always used database 0
    let mut ctx = Ctx::new("database");
    if ctx.ep.database != 0 {
        return; // the test URL selects a database already: the configuration sets it
    }
    let mut db1 = OtherDb::open(&ctx, 1);
    let src = db1.key(&ctx, "src");
    let t0 = db1.key(&ctx, "t0");
    let all = packages("db", 20);
    let _: i64 = redis::cmd("RPUSH")
        .arg(&src)
        .arg(&all)
        .query(&mut db1.con)
        .unwrap();
    let client = ctx.client_yaml("t0", &t0, "    database: 1\n");
    let cfg = ctx.config(&src, 2, "replicant", "database: 1\n", &[client]);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_for("the packages in database 1", |_| {
        db1.llen(&src) == 0 && db1.llen(&t0) == all.len()
    });
    p.stop(&ctx.pid_file());
    let mut got = db1.list(&t0);
    got.sort();
    assert_eq!(got, all);
    assert_eq!(ctx.llen(&t0), 0, "a package went to database 0");
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

#[test]
fn username_and_special_password() {
    // M1F-7 and M3-i: a Redis 6 user whose password holds every character the URL of 1.0.8
    // broke on ('#', '/', '?' panicked, '%41' was read as 'A', a tab did not connect)
    let mut ctx = Ctx::new("acl");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let password = "p@ss:w/rd#?%41\tx y";
    let user = format!("rmtest_{}", std::process::id());
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
    let all = packages("acl", 20);
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
    let cfg = ctx.config(&src, 2, "replicant", "", &[client]);
    let yaml = std::fs::read_to_string(&cfg).unwrap();
    let root_password = format!("password: {}\n", q(&ctx.ep.password));
    let yaml = yaml.replacen(&root_password, &access, 1);
    std::fs::write(&cfg, yaml).unwrap();
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, all.len())]);
    p.stop(&ctx.pid_file());
    let mut got = ctx.list(&t0);
    got.sort();
    assert_eq!(got, all);
    assert!(!p.stderr().contains("Couldn't connect"), "{}", p.stderr());
}

#[test]
fn tls_target() {
    // M3R-4 (PKG): a target over TLS, the server's certificate signed by a CA the program
    // is told to trust. Runs only where a TLS Redis is given: RM_TEST_REDIS_TLS_URL
    // (rediss://host:port, no password) and RM_TEST_REDIS_TLS_CA (the CA file).
    let (Ok(url), Ok(ca)) = (
        std::env::var("RM_TEST_REDIS_TLS_URL"),
        std::env::var("RM_TEST_REDIS_TLS_CA"),
    ) else {
        eprintln!("tls_target: skipped, RM_TEST_REDIS_TLS_URL or RM_TEST_REDIS_TLS_CA not set");
        return;
    };
    let hostport = url
        .strip_prefix("rediss://")
        .expect("RM_TEST_REDIS_TLS_URL starts with rediss://");
    let (host, port) = hostport.trim_end_matches('/').rsplit_once(':').unwrap();
    let mut ctx = Ctx::new("tls");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let all = packages("tls", 20);
    ctx.push(&src, &all);
    let client = format!(
        "  - name: 't0'\n    ssl: true\n    hostname: {}\n    port: {}\n    password: ''\n    channel: {}\n",
        q(host),
        port,
        q(&t0)
    );
    let cfg = ctx.config(&src, 2, "replicant", "", &[client]);
    let mut p = ctx.spawn_env(&cfg, "SSL_CERT_FILE", &ca);
    ctx.wait_for("the source drained", |c| c.llen(&src) == 0);
    p.stop(&ctx.pid_file());
    // Read back over TLS with the same CA
    let out = std::process::Command::new("redis-cli")
        .args([
            "--tls", "--cacert", &ca, "-h", host, "-p", port, "LRANGE", &t0, "0", "-1",
        ])
        .output()
        .expect("redis-cli to read the TLS target");
    let mut got: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect();
    let _ = std::process::Command::new("redis-cli")
        .args(["--tls", "--cacert", &ca, "-h", host, "-p", port, "DEL", &t0])
        .output();
    got.sort();
    assert_eq!(got, all, "{}", p.stderr());
}
