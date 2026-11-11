//! Unit tests of the program (kept out of main.rs so they are not counted as source)

use super::*;

fn client(name: &str, channel: &str) -> ClientConfig {
    ClientConfig {
        name: name.to_string(),
        ssl: None,
        hostname: "127.0.0.1".to_string(),
        port: 6379,
        password: String::new(),
        channel: channel.to_string(),
        timelimit: None,
        checklimit: None,
        softlimit: None,
        hardlimit: None,
        deleteblock: None,
        filter: None,
        filter_until: None,
        filter_limit: None,
        filter_replace: None,
    }
}

fn config() -> Config {
    Config {
        name: "source".to_string(),
        ssl: None,
        hostname: "127.0.0.1".to_string(),
        port: 6379,
        password: String::new(),
        channel: "src".to_string(),
        children: 1,
        mode: "replicant".to_string(),
        pid: None,
        status: None,
        filter: None,
        filter_until: None,
        filter_limit: None,
        filter_replace: None,
        ordering: None,
        ordering_buffer_time: None,
        ordering_limit: None,
        clients: vec![client("t1", "dst")],
    }
}

fn filter(
    re: Option<&str>,
    until: Option<&str>,
    limit: Option<usize>,
    replace: Option<&str>,
    data: &str,
) -> Option<String> {
    match match_filter(
        re.map(|r| Regex::new(r).unwrap()),
        until.map(str::to_string),
        limit,
        replace.map(str::to_string),
        data.to_string(),
    ) {
        MatchAnswer::Box(s) => Some(s),
        MatchAnswer::Ok(false) => None,
        MatchAnswer::Ok(true) => panic!("match_filter returned Ok(true)"),
        MatchAnswer::Err(e) => panic!("match_filter returned Err({})", e),
    }
}

// --- YAML parsing (pins what configuration files of earlier versions parse to) ---

const YAML_BASE: &str = "name: s\nhostname: h\nport: 6379\npassword: x\nchannel: c\nchildren: 1\nmode: replicant\nclients:\n  - name: t\n    hostname: h\n    port: 6379\n    password: x\n    channel: d\n";

fn yaml(extra_root: &str) -> Result<Config, String> {
    parse_yaml(&format!("{}{}", extra_root, YAML_BASE))
}

fn yaml_replace(from: &str, to: &str) -> Result<Config, String> {
    assert!(YAML_BASE.contains(from), "{} not in the base", from);
    parse_yaml(&YAML_BASE.replacen(from, to, 1))
}

#[test]
fn yaml_minimal_config() {
    let c = yaml("").unwrap();
    assert_eq!(
        (c.name.as_str(), c.port, c.password.as_str()),
        ("s", 6379, "x")
    );
    assert_eq!(
        (c.ssl, c.filter.clone(), c.ordering_limit),
        (None, None, None)
    );
    assert_eq!(c.clients.len(), 1);
    assert_eq!(c.clients[0].channel, "d");
}

#[test]
fn yaml_scalars_as_earlier_versions_read_them() {
    assert_eq!(yaml("ssl: true\n").unwrap().ssl, Some(true));
    assert_eq!(yaml("ssl: false\n").unwrap().ssl, Some(false));
    // serde_yaml 0.8 rejected the YAML 1.1 spelling "True"; it is accepted now
    assert_eq!(yaml("ssl: True\n").unwrap().ssl, Some(true));
    assert_eq!(
        yaml_replace("password: x\nchannel: c", "password: 1234\nchannel: c")
            .unwrap()
            .password,
        "1234"
    );
    assert_eq!(yaml_replace("name: s\n", "name: 1\n").unwrap().name, "1");
    assert_eq!(
        yaml_replace(
            "port: 6379\npassword: x\nchannel: c",
            "port: 0x18EB\npassword: x\nchannel: c"
        )
        .unwrap()
        .port,
        6379
    );
    assert_eq!(
        yaml("filter: 'a'\nfilter_limit: 11\n")
            .unwrap()
            .filter_limit,
        Some(11)
    );
    assert_eq!(
        yaml("ordering: '.*\"ts\": *(?P<ts>\\d+),.*#'\n")
            .unwrap()
            .ordering
            .unwrap(),
        ".*\"ts\": *(?P<ts>\\d+),.*#"
    );
    assert_eq!(yaml("unknown_key: 1\n").unwrap().name, "s");
}

#[test]
fn yaml_empty_password() {
    let empty = yaml_replace("password: x\nchannel: c", "password:\nchannel: c").unwrap();
    let tilde = yaml_replace("password: x\nchannel: c", "password: ~\nchannel: c").unwrap();
    // serde_yaml 0.8 read an empty value as the literal "~" and sent AUTH ~ with it
    assert_eq!(
        (empty.password.as_str(), tilde.password.as_str()),
        ("", "~")
    );
    let quoted = yaml_replace("password: x\nchannel: c", "password: ''\nchannel: c").unwrap();
    assert_eq!(quoted.password, "");
}

#[test]
fn yaml_empty_name_and_channel_read_as_in_earlier_versions() {
    // M3R-1 rows a and b: serde_yaml 0.8 read an empty `name:` or `channel:` (root or
    // client, a trailing comment included) as "~"; files with one ran on 1.0.8
    let c = yaml_replace("name: s\n", "name:\n").unwrap();
    assert_eq!(c.name, "~");
    assert!(verify_config(c).is_ok());
    assert_eq!(
        yaml_replace("name: s\n", "name: # label\n").unwrap().name,
        "~"
    );
    let c = yaml_replace("channel: c\n", "channel:\n").unwrap();
    assert_eq!(c.channel, "~");
    assert!(verify_config(c).is_ok());
    let c = yaml_replace("    channel: d\n", "    channel:\n").unwrap();
    assert_eq!(c.clients[0].channel, "~");
    assert_eq!(
        yaml_replace("  - name: t\n", "  - name:\n")
            .unwrap()
            .clients[0]
            .name,
        "~"
    );
    // Quoted empty strings stay empty and are rejected, as in 1.0.8
    let c = yaml_replace("name: s\n", "name: ''\n").unwrap();
    assert_eq!(c.name, "");
    assert!(verify_config(c).is_err());
    assert_eq!(
        yaml_replace("channel: c\n", "channel: \"\"\n")
            .unwrap()
            .channel,
        ""
    );
    // `~` and `null` read as text, as in 1.0.8
    assert_eq!(yaml_replace("name: s\n", "name: ~\n").unwrap().name, "~");
    assert_eq!(
        yaml_replace("name: s\n", "name: null\n").unwrap().name,
        "null"
    );
}

#[test]
fn yaml_declared_differences_with_earlier_versions() {
    // M3R-1 rows c to f: what serde_yaml 0.8 (1.0.8) read and serde_norway reads now.
    // c: a control character (form feed, or U+0092 in a comment): 0.8 kept it, now an error
    assert!(yaml_replace("name: s\n", "name: s\u{c}\n").is_err());
    assert!(yaml_replace("name: s\n", "name: s # it\u{92}s\n").is_err());
    // d: a value on the line after its key at column 0: 0.8 read it, now an error
    assert!(yaml_replace("channel: c\n", "channel:\nc\n").is_err());
    // e: `Null` on an optional text key: 0.8 read the text "Null", now absent
    assert_eq!(yaml("filter: Null\n").unwrap().filter, None);
    // f: an empty `hostname:` read "~" in 0.8 (never connected); now "" and rejected
    let c = yaml_replace("hostname: h\n", "hostname:\n").unwrap();
    assert_eq!(c.hostname, "");
    assert!(verify_config(c).is_err());
}

#[test]
fn yaml_rejections() {
    assert!(yaml("ssl: yes\n").is_err());
    assert!(yaml_replace(
        "port: 6379\npassword: x\nchannel: c",
        "port: '6379'\npassword: x\nchannel: c"
    )
    .is_err());
    assert!(yaml("filter: 'a'\nfilter_limit: 1.0\n").is_err());
    assert!(yaml("name: twice\n").is_err());
    assert!(yaml_replace("children: 1", "children: 70000").is_err());
    assert!(yaml_replace("channel: c\n", "").is_err());
}

#[test]
fn yaml_readme_example() {
    let readme = include_str!("../README.md");
    let start = readme.find("## Full example configuration").unwrap();
    let body = &readme[start..];
    let begin = body.find("```yaml\n").unwrap() + "```yaml\n".len();
    let end = begin + body[begin..].find("```").unwrap();
    let c = parse_yaml(&body[begin..end]).unwrap();
    assert_eq!(
        (c.name.as_str(), c.children, c.mode.as_str()),
        ("Source", 2, "replicant")
    );
    assert_eq!(c.ordering_buffer_time, Some(30));
    assert_eq!(c.clients.len(), 2);
    assert_eq!(c.clients[0].filter.as_deref(), Some("^(1|3|5|7|9)#"));
    assert_eq!(c.clients[0].filter_replace.as_deref(), Some(""));
    assert_eq!(c.clients[1].deleteblock, None);
    assert!(verify_config(c).is_ok());
}

// --- redis commands ---

#[test]
fn blpop_sends_an_integer_timeout() {
    assert_eq!(
        blpop_cmd("src").get_packed_command(),
        b"*3\r\n$5\r\nBLPOP\r\n$3\r\nsrc\r\n$1\r\n1\r\n".to_vec()
    );
}

// --- match_filter ---

#[test]
fn filter_without_regex_returns_input() {
    assert_eq!(filter(None, None, None, None, "abc"), Some("abc".into()));
}

#[test]
fn filter_match_without_replace_returns_input() {
    assert_eq!(
        filter(Some("b"), None, None, None, "abc"),
        Some("abc".into())
    );
}

#[test]
fn filter_no_match_drops() {
    assert_eq!(filter(Some("z"), None, None, None, "abc"), None);
}

#[test]
fn filter_readme_example() {
    let out = filter(Some("ell"), Some("r"), Some(11), Some("ELL"), "Hello world");
    assert_eq!(out, Some("HELLo world".into()));
}

#[test]
fn filter_limit_cuts_haystack() {
    assert_eq!(
        filter(Some("world"), None, Some(5), None, "Hello world"),
        None
    );
    assert_eq!(
        filter(Some("Hello"), None, Some(5), None, "Hello world"),
        Some("Hello world".into())
    );
}

#[test]
fn filter_limit_zero_means_no_limit() {
    assert_eq!(
        filter(Some("world"), None, Some(0), None, "Hello world"),
        Some("Hello world".into())
    );
}

#[test]
fn filter_limit_longer_than_input() {
    assert_eq!(
        filter(Some("d$"), None, Some(1000), None, "Hello world"),
        Some("Hello world".into())
    );
}

#[test]
fn filter_until_excludes_delimiter_current_behaviour() {
    // A-C3: the delimiter is not part of the haystack, so a regex that includes it
    // never matches (the README client example). M3 decides the fix.
    assert_eq!(
        filter(
            Some("^(1|3|5|7|9)#"),
            Some("#"),
            Some(100),
            Some(""),
            "1#payload"
        ),
        None
    );
    assert_eq!(
        filter(
            Some("^(1|3|5|7|9)$"),
            Some("#"),
            Some(100),
            None,
            "1#payload"
        ),
        Some("1#payload".into())
    );
}

#[test]
fn filter_until_not_found_uses_whole_slice() {
    assert_eq!(
        filter(Some("payload"), Some("#"), None, None, "1 payload"),
        Some("1 payload".into())
    );
}

#[test]
fn filter_empty_until_is_ignored() {
    assert_eq!(
        filter(Some("world"), Some(""), None, None, "Hello world"),
        Some("Hello world".into())
    );
}

#[test]
fn filter_match_after_until_drops() {
    assert_eq!(
        filter(Some("world"), Some(" "), None, None, "Hello world"),
        None
    );
}

#[test]
fn filter_replace_first_match_only_and_keeps_tail() {
    let out = filter(Some("a"), Some("#"), None, Some("X"), "aaa#aaa");
    assert_eq!(out, Some("Xaa#aaa".into()));
}

#[test]
fn filter_replace_with_groups() {
    let out = filter(Some(r"^(\d)-(\d)"), None, None, Some("$2-$1"), "1-2 rest");
    assert_eq!(out, Some("2-1 rest".into()));
}

#[test]
#[should_panic]
fn filter_limit_inside_multibyte_char_panics_until_m3() {
    // A-C2: slicing a &str inside a multibyte character panics. M3 moves to bytes.
    filter(Some("x"), None, Some(1), None, "\u{e9}x");
}

// --- match_ordering ---

#[test]
fn ordering_package_without_ts_is_released_at_once() {
    let mut heap = BinaryHeap::new();
    let out = match_ordering(None, Some(30), Some("p".into()), &mut heap);
    assert_eq!(out, vec!["p".to_string()]);
    assert!(heap.is_empty());
}

#[test]
fn ordering_without_buffer_time_releases_at_once() {
    let mut heap = BinaryHeap::new();
    let out = match_ordering(Some(5), None, Some("p".into()), &mut heap);
    assert_eq!(out, vec!["p".to_string()]);
}

#[test]
fn ordering_retains_within_buffer_time() {
    let mut heap = BinaryHeap::new();
    let out = match_ordering(Some(5), Some(30), Some("p".into()), &mut heap);
    assert!(out.is_empty());
    assert_eq!(heap.len(), 1);
    let out = match_ordering(None, Some(30), None, &mut heap);
    assert!(out.is_empty());
}

#[test]
fn ordering_releases_old_entries_in_ts_order() {
    let mut heap = BinaryHeap::new();
    for (ts, data) in [(5u128, "five"), (3, "three"), (9, "nine")] {
        heap.push(Reverse((ts, (0u64, data.to_string()))));
    }
    let out = match_ordering(None, Some(30), None, &mut heap);
    assert_eq!(out, vec!["three", "five", "nine"]);
    assert!(heap.is_empty());
}

#[test]
fn ordering_recent_entry_with_lower_ts_blocks_the_rest() {
    let mut heap = BinaryHeap::new();
    for (ts, data) in [(5u128, "five"), (3, "three"), (9, "nine")] {
        heap.push(Reverse((ts, (0u64, data.to_string()))));
    }
    let out = match_ordering(Some(7), Some(30), Some("seven".into()), &mut heap);
    assert_eq!(out, vec!["three", "five"]);
    assert_eq!(heap.len(), 2);
}

// --- can_check_queue ---

#[test]
fn check_queue_default_interval_without_limits() {
    assert!(can_check_queue(None, None, 0, 0));
    assert!(!can_check_queue(None, None, 0, get_current_time()));
}

#[test]
fn check_queue_timelimit() {
    let now = get_current_time();
    assert!(can_check_queue(Some(5), None, 1, now - 10));
    assert!(!can_check_queue(Some(5), None, 1, now));
}

#[test]
fn check_queue_checklimit() {
    let now = get_current_time();
    assert!(can_check_queue(None, Some(100), 0, now));
    assert!(!can_check_queue(None, Some(100), 3, now));
    assert!(can_check_queue(Some(3600), Some(100), 0, now));
}

// --- verify_config ---

fn err(c: Config) -> String {
    verify_config(c).expect_err("config should be rejected")
}

#[test]
fn verify_accepts_minimal_config() {
    assert!(verify_config(config()).is_ok());
}

#[test]
fn verify_rejects_unknown_mode() {
    let mut c = config();
    c.mode = "broadcast".into();
    assert!(err(c).contains("Mode 'broadcast' is unknown"));
}

#[test]
fn verify_accepts_spreader() {
    let mut c = config();
    c.mode = "spreader".into();
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_rejects_zero_children() {
    let mut c = config();
    c.children = 0;
    assert!(err(c).contains("Children"));
}

#[test]
fn verify_rejects_empty_source_fields() {
    let mut c = config();
    c.name = String::new();
    assert!(err(c).contains("empty name"));
    let mut c = config();
    c.hostname = String::new();
    assert!(err(c).contains("empty hostname"));
    let mut c = config();
    c.channel = String::new();
    assert!(err(c).contains("empty channel"));
}

#[test]
fn verify_rejects_filter_options_without_filter() {
    let mut c = config();
    c.filter_until = Some("#".into());
    assert!(err(c).contains("filter is not defined"));
    let mut c = config();
    c.clients[0].filter_replace = Some("x".into());
    assert!(err(c).contains("filter is not defined"));
}

#[test]
fn verify_rejects_empty_filter() {
    let mut c = config();
    c.filter = Some(String::new());
    assert!(err(c).contains("filter can not empty"));
}

#[test]
fn verify_accepts_complete_ordering() {
    let mut c = config();
    c.ordering = Some("(?P<ts>\\d+)".into());
    c.ordering_buffer_time = Some(1);
    c.ordering_limit = Some(100);
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_rejects_single_ordering_option() {
    let mut c = config();
    c.ordering_buffer_time = Some(1);
    assert!(err(c).contains("ordering"));
}

#[test]
fn verify_accepts_two_of_three_ordering_options_current_behaviour() {
    // M1F-5: the check is `configured < 2`, so two of the three options pass.
    let mut c = config();
    c.ordering = Some("(?P<ts>\\d+)".into());
    c.ordering_buffer_time = Some(1);
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_limits_all_or_nothing() {
    let mut c = config();
    c.clients[0].hardlimit = Some(10);
    assert!(err(c).contains("must set all limits"));
    let mut c = config();
    c.clients[0].timelimit = Some(1);
    c.clients[0].checklimit = Some(1);
    c.clients[0].softlimit = Some(5);
    c.clients[0].hardlimit = Some(10);
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_rejects_target_equal_to_source() {
    let mut c = config();
    c.clients[0].channel = "src".into();
    assert!(err(c).contains("same connection information"));
}

#[test]
fn verify_rejects_no_clients() {
    let mut c = config();
    c.clients.clear();
    assert!(err(c).contains("No clients"));
}

#[test]
fn verify_rejects_empty_client_channel() {
    let mut c = config();
    c.clients[0].channel = String::new();
    assert!(err(c).contains("empty channel"));
}

#[test]
fn queuer_flushes_the_ordering_buffer_for_a_request_after_the_stop_order() {
    // M2R-1: main sends `true` to the queuer before telling the children to stop, so a
    // child's first request after its stop order must get the whole ordering buffer.
    // The buggy queuer answers it from match_ordering when parked in its wait, so a few
    // rounds make the failure certain; on fixed code every round is deterministic.
    for round in 0..50 {
        let mut c = config();
        c.ordering_buffer_time = Some(30);
        let (keep_tx, keep_rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel();
        let (ans_tx, ans_rx) = mpsc::channel();
        let (stat_tx, stat_rx) = mpsc::channel();
        let handle = thread::spawn(move || queuer(true, c, keep_rx, req_rx, vec![ans_tx], stat_tx));

        req_tx.send((0, Some(1), Some("p1".to_string()))).unwrap();
        assert!(
            ans_rx.recv().unwrap().is_empty(),
            "round {round}: held for 30 s"
        );
        // Let the queuer park in its wait, where M2R-1 happens
        thread::sleep(Duration::from_millis(5));
        keep_tx.send(true).unwrap();
        req_tx.send((0, None, None)).unwrap();
        assert_eq!(
            ans_rx.recv().unwrap(),
            vec!["p1".to_string()],
            "round {round}"
        );

        keep_tx.send(false).unwrap();
        handle.join().unwrap();
        let sizes: Vec<Option<usize>> = stat_rx.try_iter().collect();
        assert_eq!(sizes, vec![Some(1), Some(0), None], "round {round}");
    }
}

#[test]
fn verify_accepts_empty_client_name_current_behaviour() {
    // M1F-6: the client hostname is checked twice and the name never.
    let mut c = config();
    c.clients[0].name = String::new();
    assert!(verify_config(c).is_ok());
}
