//! Unit tests of the program (kept out of main.rs so they are not counted as source)

use super::*;

fn client(name: &str, channel: &str) -> ClientConfig {
    ClientConfig {
        name: name.to_string(),
        ssl: None,
        hostname: "127.0.0.1".to_string(),
        port: 6379,
        password: String::new(),
        username: None,
        database: None,
        connect_timeout: None,
        command_timeout: None,
        channel: channel.to_string(),
        timelimit: None,
        checklimit: None,
        softlimit: None,
        hardlimit: None,
        deleteblock: None,
        filter: None,
        filter_until: None,
        filter_until_included: None,
        filter_limit: None,
        filter_replace: None,
        filter_replace_header: None,
    }
}

fn config() -> Config {
    Config {
        name: "source".to_string(),
        ssl: None,
        hostname: "127.0.0.1".to_string(),
        port: 6379,
        password: String::new(),
        username: None,
        database: None,
        connect_timeout: None,
        command_timeout: None,
        channel: "src".to_string(),
        children: 1,
        mode: "replicant".to_string(),
        pid: None,
        status: None,
        filter: None,
        filter_until: None,
        filter_until_included: None,
        filter_limit: None,
        filter_replace: None,
        filter_replace_header: None,
        ordering: None,
        ordering_buffer_time: None,
        ordering_limit: None,
        clients: vec![client("t1", "dst")],
    }
}

/// A filter_replace replacement, as the configuration key gives it
fn rx(replace: Option<&str>) -> Option<Replace> {
    replace.map(|r| Replace::Regex(r.as_bytes().to_vec()))
}

/// A filter_replace_header filter
fn header(
    re: &str,
    until: Option<&str>,
    included: bool,
    limit: Option<usize>,
    literal: &str,
    data: &[u8],
) -> Option<Vec<u8>> {
    Filter::new(
        re,
        until,
        included,
        limit,
        Some(Replace::Header(literal.as_bytes().to_vec())),
    )
    .unwrap()
    .apply(data)
    .map(|c| c.into_owned())
}

fn filter_bytes(
    re: Option<&str>,
    until: Option<&str>,
    limit: Option<usize>,
    replace: Option<&str>,
    data: &[u8],
) -> Option<Vec<u8>> {
    match re {
        None => Some(data.to_vec()),
        Some(r) => Filter::new(r, until, false, limit, rx(replace))
            .unwrap()
            .apply(data)
            .map(|c| c.into_owned()),
    }
}

fn filter(
    re: Option<&str>,
    until: Option<&str>,
    limit: Option<usize>,
    replace: Option<&str>,
    data: &str,
) -> Option<String> {
    filter_bytes(re, until, limit, replace, data.as_bytes()).map(|v| String::from_utf8(v).unwrap())
}

fn strs(list: Vec<Vec<u8>>) -> Vec<String> {
    list.into_iter()
        .map(|v| String::from_utf8(v).unwrap())
        .collect()
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
    assert_eq!(
        yaml("filter: 'a'\nfilter_until_included: true\n")
            .unwrap()
            .filter_until_included,
        Some(true)
    );
    assert_eq!(yaml("").unwrap().filter_until_included, None);
    assert_eq!(
        yaml("filter: 'a'\nfilter_replace_header: ''\n")
            .unwrap()
            .filter_replace_header,
        Some(String::new())
    );
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
fn yaml_unknown_clients_key_inside_a_client_keeps_the_empty_name() {
    // M3R-7: an unknown key `clients` inside a client is ignored by the typed read; it
    // must not turn the "~" of an empty `name:` or `channel:` off for the whole file
    for unknown in ["1", "x", "{a: 1}", "[1, 2]"] {
        let text = YAML_BASE.replace("name: s\n", "name:\n").replace(
            "    channel: d\n",
            &format!("    channel:\n    clients: {}\n", unknown),
        );
        let c = parse_yaml(&text).unwrap();
        assert_eq!(
            (c.name.as_str(), c.clients[0].channel.as_str()),
            ("~", "~"),
            "{}",
            unknown
        );
    }
}

#[test]
fn yaml_declared_differences_with_earlier_versions() {
    // M3R-1 rows c to f: what serde_yaml 0.8 (1.0.8) read and serde_norway reads now.
    // c: a control character (form feed, or U+0092 in a comment): 0.8 kept it, now an error
    assert!(yaml_replace("name: s\n", "name: s\u{c}\n").is_err());
    assert!(yaml_replace("name: s\n", "name: s # it\u{92}s\n").is_err());
    // d: a value on the line after its key at column 0: 0.8 read it, now an error
    assert!(yaml_replace("channel: c\n", "channel:\nc\n").is_err());
    // e: `Null` or `NULL` on an optional text key: 0.8 read the text, now absent
    assert_eq!(yaml("filter: Null\n").unwrap().filter, None);
    assert_eq!(yaml("filter: NULL\n").unwrap().filter, None);
    // f: an empty `hostname:` or `mode:` read "~" in 0.8 (never worked); now "" and rejected
    let c = yaml_replace("hostname: h\n", "hostname:\n").unwrap();
    assert_eq!(c.hostname, "");
    assert!(verify_config(c).is_err());
    let c = yaml_replace("mode: replicant\n", "mode:\n").unwrap();
    assert_eq!(c.mode, "");
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
    let text = &body[begin..end];
    // Every key of the example is one the program knows
    assert_eq!(unknown_keys(text), Vec::<String>::new());
    let c = parse_yaml(text).unwrap();
    assert_eq!(
        (c.name.as_str(), c.children, c.mode.as_str()),
        ("Source", 2, "replicant")
    );
    assert_eq!((c.ssl, c.database), (Some(false), Some(0)));
    assert_eq!(
        (c.connect_timeout, c.command_timeout),
        (Some(2.0), Some(5.0))
    );
    assert_eq!(c.clients[0].ssl, Some(true));
    assert_eq!(c.clients[0].command_timeout, Some(1.5));
    assert_eq!(c.ordering_buffer_time, Some(30));
    assert_eq!(c.clients.len(), 2);
    assert_eq!(c.clients[0].filter.as_deref(), Some("^(1|3|5|7|9)#"));
    assert_eq!(c.clients[0].filter_replace.as_deref(), Some(""));
    assert_eq!(c.clients[0].filter_until_included, Some(true));
    // The example strips the routing prefix of the packages it routes
    let filters = build_filters(&c).unwrap();
    let odd = filters.clients[0].as_ref().unwrap();
    assert_eq!(odd.apply(b"3#data").unwrap().as_ref(), b"data");
    assert!(odd.apply(b"4#data").is_none());
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
fn filter_until_excludes_delimiter_by_default() {
    // A-C3: without filter_until_included the delimiter is not part of the header, so a
    // regex that includes it never matches (1.0 behaviour, kept as the default).
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

fn included(
    re: &str,
    until: &str,
    limit: Option<usize>,
    replace: Option<&str>,
    data: &str,
) -> Option<String> {
    Filter::new(re, Some(until), true, limit, rx(replace))
        .unwrap()
        .apply(data.as_bytes())
        .map(|c| String::from_utf8(c.into_owned()).unwrap())
}

#[test]
fn filter_until_included_puts_the_delimiter_in_the_header() {
    // M3-b: the README client example matches and strips the routing prefix
    assert_eq!(
        included("^(1|3|5|7|9)#", "#", Some(100), Some(""), "1#payload"),
        Some("payload".into())
    );
    assert_eq!(
        included("^(1|3|5|7|9)#", "#", Some(100), Some(""), "2#payload"),
        None
    );
    // A pattern anchored before the delimiter no longer matches with it included
    assert_eq!(
        included("^(1|3|5|7|9)$", "#", Some(100), None, "1#payload"),
        None
    );
    assert_eq!(
        included("^(1|3|5|7|9)#$", "#", Some(100), None, "1#payload"),
        Some("1#payload".into())
    );
}

#[test]
fn filter_until_included_edges() {
    // Multi-byte delimiter, delimiter not found, delimiter cut by the limit
    assert_eq!(
        included("^ab::$", "::", None, Some("X"), "ab::cd::ef"),
        Some("Xcd::ef".into())
    );
    assert_eq!(included("cd", "#", None, None, "abcd"), Some("abcd".into()));
    assert_eq!(included("^ab#$", "#", Some(2), None, "ab#cd"), None);
    assert_eq!(
        included("^ab$", "#", Some(2), None, "ab#cd"),
        Some("ab#cd".into())
    );
}

#[test]
fn verify_rejects_until_included_without_filter() {
    let mut c = config();
    c.clients[0].filter_until_included = Some(true);
    assert!(err(c).contains("Client 't1' sets filter_until_included without filter"));
    let mut c = config();
    c.filter_until_included = Some(false);
    assert!(err(c).contains("Source 'source' sets filter_until_included without filter"));
}

#[test]
fn filter_replace_header_replaces_the_whole_header() {
    // M3-a: the literal replaces everything before the cut, not only the regex match
    assert_eq!(
        header("^route\\d+$", Some(":"), false, None, "X", b"route12:abc").unwrap(),
        b"X:abc".to_vec()
    );
    assert_eq!(
        header("^route\\d+:$", Some(":"), true, None, "", b"route12:abc").unwrap(),
        b"abc".to_vec()
    );
    // No regex groups: "$1" is literal
    assert_eq!(
        header("^(r)", Some(":"), true, None, "$1-", b"r:abc").unwrap(),
        b"$1-abc".to_vec()
    );
    // Header cut by the limit only; the rest of the package, bytes included, is kept
    assert_eq!(
        header("^ab", None, false, Some(2), "Z", b"abc\xff").unwrap(),
        b"Zc\xff".to_vec()
    );
    assert_eq!(
        header("^zz", Some(":"), true, None, "X", b"route12:abc"),
        None
    );
}

#[test]
fn replacement_from_the_configuration_keys() {
    assert!(replacement(&None, &None).is_none());
    assert!(matches!(replacement(&Some("a".into()), &None), Some(Replace::Regex(r)) if r == b"a"));
    assert!(matches!(replacement(&None, &Some("h".into())), Some(Replace::Header(h)) if h == b"h"));
}

#[test]
fn verify_rejects_both_replacements() {
    let mut c = config();
    c.clients[0].filter = Some("a".into());
    c.clients[0].filter_replace = Some("b".into());
    c.clients[0].filter_replace_header = Some("c".into());
    assert!(err(c).contains("Client 't1' sets both filter_replace and filter_replace_header"));
    let mut c = config();
    c.filter = Some("a".into());
    c.filter_replace = Some("b".into());
    c.filter_replace_header = Some(String::new());
    assert!(err(c).contains("Source 'source' sets both filter_replace and filter_replace_header"));
}

#[test]
fn verify_rejects_replace_header_without_filter() {
    let mut c = config();
    c.clients[0].filter_replace_header = Some("x".into());
    assert!(err(c).contains("Client 't1' sets filter_replace_header without filter"));
    let mut c = config();
    c.filter_replace_header = Some("x".into());
    assert!(err(c).contains("Source 'source' sets filter_replace_header without filter"));
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
fn filter_limit_inside_multibyte_char_cuts_bytes() {
    // A-C2 fixed: the limit counts bytes and may cut a multibyte character without panicking.
    // "\u{e9}" is two bytes, so a limit of 1 leaves half of it and "x" out of the haystack.
    assert_eq!(filter(Some("x"), None, Some(1), None, "\u{e9}x"), None);
    assert_eq!(filter(Some("\u{e9}"), None, Some(1), None, "\u{e9}x"), None);
    assert_eq!(
        filter(Some("\u{e9}"), None, Some(2), None, "\u{e9}x"),
        Some("\u{e9}x".into())
    );
}

#[test]
fn filter_passes_non_utf8_payloads() {
    // A-C1 fixed: only the header has to match; the payload is carried as bytes.
    let data = b"k#\xff\xfe\x00tail";
    assert_eq!(
        filter_bytes(Some("^k$"), Some("#"), None, None, data),
        Some(data.to_vec())
    );
    assert_eq!(
        filter_bytes(Some("^k"), None, None, Some("K"), data),
        Some(b"K#\xff\xfe\x00tail".to_vec())
    );
    assert_eq!(filter_bytes(Some("^z"), None, None, None, data), None);
}

#[test]
fn filter_unicode_classes_do_not_match_invalid_utf8() {
    // regex::bytes in Unicode mode: `.` never matches an invalid byte; `(?-u)` opts out
    assert_eq!(filter_bytes(Some("^.$"), None, None, None, b"\xff"), None);
    assert_eq!(
        filter_bytes(Some("(?-u)^\\xff"), None, None, None, b"\xff\x01"),
        Some(b"\xff\x01".to_vec())
    );
    assert_eq!(
        filter_bytes(Some("^.$"), None, None, None, "\u{e9}".as_bytes()),
        Some("\u{e9}".as_bytes().to_vec())
    );
}

#[test]
fn filter_borrows_the_package_unless_it_replaces() {
    let data = b"Hello world";
    let f = Filter::new("ell", Some("r"), false, Some(11), None).unwrap();
    assert!(matches!(f.apply(data), Some(Cow::Borrowed(_))));
    let f = Filter::new("ell", Some("r"), false, Some(11), rx(Some("ELL"))).unwrap();
    assert!(matches!(f.apply(data), Some(Cow::Owned(_))));
}

#[test]
fn filter_clone_matches_like_the_original() {
    // Each worker gets a clone (its own regex cache); it must behave the same
    let f = Filter::new(r"^(\d)-(\d)", Some(" "), false, Some(10), rx(Some("$2-$1"))).unwrap();
    let g = f.clone();
    assert_eq!(f.apply(b"1-2 rest"), g.apply(b"1-2 rest"));
    assert_eq!(g.apply(b"1-2 rest").unwrap().as_ref(), b"2-1 rest");
}

#[test]
fn verify_rejects_a_client_regex_that_does_not_compile() {
    // M1F-8: checked at start instead of panicking every worker
    let mut c = config();
    c.clients[0].filter = Some("(".into());
    let e = err(c);
    assert!(
        e.contains("Client 't1' has a filter Regex '(' that doesn't compile"),
        "{}",
        e
    );
}

#[test]
fn verify_rejects_a_root_regex_that_does_not_compile() {
    let mut c = config();
    c.filter = Some("[".into());
    let e = err(c);
    assert!(e.contains("Filter Regex '[' doesn't compile"), "{}", e);
}

#[test]
fn build_filters_keeps_client_order() {
    let mut c = config();
    c.clients.push(client("t2", "dst2"));
    c.clients[1].filter = Some("^b".into());
    let f = build_filters(&c).unwrap();
    assert!(f.root.is_none());
    assert_eq!(f.clients.len(), 2);
    assert!(f.clients[0].is_none());
    assert!(f.clients[1].as_ref().unwrap().apply(b"b").is_some());
}

// --- ordering buffer ---

fn held(entries: &[(u128, &str, Instant)]) -> OrderState {
    let buffer = OrderBuffer::new();
    for (ts, data, at) in entries {
        buffer.push(*ts, data.as_bytes().to_vec(), *at);
    }
    buffer.state.into_inner().unwrap()
}

#[test]
fn ordering_without_buffer_time_releases_at_once() {
    let t0 = Instant::now();
    let mut st = held(&[(5, "p", t0)]);
    assert_eq!(strs(release(&mut st, t0, Duration::ZERO)), vec!["p"]);
}

#[test]
fn ordering_retains_within_buffer_time() {
    let t0 = Instant::now();
    let mut st = held(&[(5, "p", t0)]);
    assert!(release(
        &mut st,
        t0 + Duration::from_secs(29),
        Duration::from_secs(30)
    )
    .is_empty());
    assert_eq!(st.heap.len(), 1);
    assert_eq!(
        next_release(&st, Duration::from_secs(30)),
        Some(t0 + Duration::from_secs(30))
    );
}

#[test]
fn ordering_release_boundary_is_in_milliseconds() {
    // M3-c = ca: released when it has waited the buffer time on a monotonic clock in
    // milliseconds. 1.0.8 compared wall-clock seconds, so a package waited between T and
    // T + 1 s (declared difference).
    let t0 = Instant::now();
    let hold = Duration::from_secs(1);
    let mut st = held(&[(5, "p", t0)]);
    assert!(release(&mut st, t0 + Duration::from_millis(999), hold).is_empty());
    assert_eq!(
        strs(release(&mut st, t0 + Duration::from_millis(1000), hold)),
        vec!["p"]
    );
}

#[test]
fn ordering_buffer_time_beyond_the_clock_waits_for_finish() {
    // M3R-13: `arrival + hold` overflowed for a buffer time past the clock's range and
    // panicked the sender with the buffer lock held. With checked_add such a package is
    // never due: nothing is released and no wake-up is scheduled until finish(), which
    // releases everything in ts order.
    let t0 = Instant::now();
    let hold = Duration::from_secs(u64::MAX);
    let mut st = held(&[(5, "five", t0), (3, "three", t0)]);
    assert!(release(&mut st, t0 + Duration::from_secs(3600), hold).is_empty());
    assert_eq!(next_release(&st, hold), None);
    st.finish = true;
    assert_eq!(
        strs(release(&mut st, t0 + Duration::from_secs(3600), hold)),
        vec!["three", "five"]
    );
}

#[test]
fn ordering_releases_old_entries_in_ts_order() {
    let t0 = Instant::now();
    let mut st = held(&[(5, "five", t0), (3, "three", t0), (9, "nine", t0)]);
    let out = release(
        &mut st,
        t0 + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    assert_eq!(strs(out), vec!["three", "five", "nine"]);
    assert!(st.heap.is_empty());
}

#[test]
fn ordering_recent_entry_with_lower_ts_blocks_the_rest() {
    let t0 = Instant::now();
    let later = t0 + Duration::from_secs(20);
    let mut st = held(&[
        (5, "five", t0),
        (3, "three", t0),
        (9, "nine", t0),
        (7, "seven", later),
    ]);
    let out = release(
        &mut st,
        t0 + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    assert_eq!(strs(out), vec!["three", "five"]);
    assert_eq!(st.heap.len(), 2);
}

#[test]
fn ordering_equal_ts_keeps_arrival_order() {
    // M3-c = ca, M3F-9: equal `ts` leave in arrival order. 1.0.8 sorted them by arrival
    // second and then by payload bytes, so "b" then "a" in the same second left as "a",
    // "b" (declared difference).
    let t0 = Instant::now();
    let mut st = held(&[(5, "b", t0), (5, "a", t0), (5, "c", t0), (4, "z", t0)]);
    let out = release(&mut st, t0, Duration::ZERO);
    assert_eq!(strs(out), vec!["z", "b", "a", "c"]);
}

#[test]
fn ordering_finish_releases_everything_in_ts_order() {
    let t0 = Instant::now();
    let mut st = held(&[(5, "five", t0), (3, "three", t0), (9, "nine", t0)]);
    st.finish = true;
    assert_eq!(
        strs(release(&mut st, t0, Duration::from_secs(30))),
        vec!["three", "five", "nine"]
    );
}

#[test]
fn ordered_sender_delivers_every_worker_package_once_in_ts_order() {
    // S3-1 b: packages pushed by several workers at once all come out through the sender,
    // each once; with a buffer longer than the run they come out in ts order on finish.
    for (workers, hold) in [
        (2u64, Duration::from_secs(60)),
        (8, Duration::from_secs(60)),
        (8, Duration::ZERO),
    ] {
        let buffer = Arc::new(OrderBuffer::new());
        let per = 2000u64;
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = out.clone();
        let b = buffer.clone();
        let sender = thread::spawn(move || {
            send_ordered(&b, hold, |batch| {
                sink.lock().unwrap().extend(batch.unwrap_or_default())
            });
        });
        let pushers: Vec<_> = (0..workers)
            .map(|w| {
                let b = buffer.clone();
                thread::spawn(move || {
                    for k in 0..per {
                        // Worker w holds ts w, w + workers, ...: every ts once overall
                        let ts = k * workers + w;
                        b.push(
                            ts as u128,
                            format!("{}:{}", w, ts).into_bytes(),
                            Instant::now(),
                        );
                    }
                })
            })
            .collect();
        for p in pushers {
            p.join().unwrap();
        }
        buffer.finish();
        sender.join().unwrap();
        let got = strs(Arc::try_unwrap(out).unwrap().into_inner().unwrap());
        assert_eq!(
            got.len() as u64,
            workers * per,
            "workers={} hold={:?}",
            workers,
            hold
        );
        let mut seen: Vec<u64> = got
            .iter()
            .map(|p| p.split(':').nth(1).unwrap().parse().unwrap())
            .collect();
        if hold > Duration::ZERO {
            let want: Vec<u64> = (0..workers * per).collect();
            assert_eq!(seen, want, "ts order");
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len() as u64, workers * per, "each package once");
    }
}

#[test]
fn ordered_sender_idle_turn_once_a_second() {
    // L4c-3, L4c-6: with nothing to deliver the sender gets one idle turn a second (its
    // idle path), not before the first second and not more often
    let buffer = Arc::new(OrderBuffer::new());
    let turns = Arc::new(Mutex::new(Vec::new()));
    let (b, log) = (buffer.clone(), turns.clone());
    let start = Instant::now();
    let sender = thread::spawn(move || {
        send_ordered(&b, Duration::ZERO, |batch| {
            log.lock().unwrap().push((batch.is_none(), start.elapsed()))
        })
    });
    thread::sleep(Duration::from_millis(2500));
    buffer.finish();
    sender.join().unwrap();
    let turns = turns.lock().unwrap().clone();
    assert_eq!(turns.len(), 2, "{:?}", turns);
    assert!(turns.iter().all(|(idle, _)| *idle), "{:?}", turns);
    assert!(turns[0].1 >= DEFAULT_CHECK, "{:?}", turns);
    assert!(turns[1].1 >= turns[0].1 + DEFAULT_CHECK, "{:?}", turns);
}

#[test]
fn ordered_sender_waits_without_spinning_and_wakes_on_push() {
    // Nothing held: the sender waits on the condition variable (an idle turn once a second
    // only, below) and a push into the empty buffer wakes it.
    let buffer = Arc::new(OrderBuffer::new());
    let out = Arc::new(Mutex::new(Vec::new()));
    let (b, sink) = (buffer.clone(), out.clone());
    let sender = thread::spawn(move || {
        send_ordered(&b, Duration::ZERO, |batch| {
            sink.lock().unwrap().extend(batch.unwrap_or_default())
        })
    });
    thread::sleep(Duration::from_millis(50));
    assert!(out.lock().unwrap().is_empty());
    buffer.push(1, b"one".to_vec(), Instant::now());
    let deadline = Instant::now() + Duration::from_secs(5);
    while out.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the push did not wake the sender"
        );
        thread::sleep(Duration::from_millis(1));
    }
    buffer.finish();
    sender.join().unwrap();
    assert_eq!(strs(out.lock().unwrap().clone()), vec!["one"]);
}

// --- ordering_ts ---

fn ts_of(limit: Option<usize>, data: &[u8]) -> Result<u128, &'static str> {
    ordering_ts(&Regex::new(r#""ts": *(?P<ts>\d+),"#).unwrap(), limit, data)
}

#[test]
fn ordering_ts_found() {
    assert_eq!(ts_of(None, br#"{"ts": 42, "x": 1}"#), Ok(42));
    assert_eq!(ts_of(Some(0), br#"{"ts": 42, "x": 1}"#), Ok(42));
}

#[test]
fn ordering_ts_in_a_non_utf8_package() {
    assert_eq!(ts_of(Some(64), b"{\"ts\": 7,\xff\xfe}"), Ok(7));
}

#[test]
fn ordering_ts_limit_inside_multibyte_char_does_not_panic() {
    // A-C2 for ordering_limit: the cut falls inside "\u{e9}" (two bytes)
    let data = "\u{e9}{\"ts\": 7,}".as_bytes();
    assert_eq!(
        ts_of(Some(1), data),
        Err("Found a package without ordering information")
    );
    assert_eq!(ts_of(Some(64), data), Ok(7));
}

#[test]
fn ordering_ts_missing_or_unparsable() {
    assert_eq!(
        ts_of(None, b"no timestamp here"),
        Err("Found a package without ordering information")
    );
    // \d in Unicode mode matches other scripts' digits, which do not parse as u128
    assert_eq!(
        ts_of(None, "{\"ts\": \u{663},}".as_bytes()),
        Err("Found a package with ordering information but 'ts' couldn't be parsed to u128")
    );
    // Past u128::MAX
    assert_eq!(
        ts_of(None, format!("{{\"ts\": {}0,}}", u128::MAX).as_bytes()),
        Err("Found a package with ordering information but 'ts' couldn't be parsed to u128")
    );
}

// --- can_check_queue ---

#[test]
fn check_queue_first_offer_checks() {
    let now = Instant::now();
    assert!(can_check_queue(None, None, 0, None, now));
    assert!(can_check_queue(Some(3600), Some(100), 5, None, now));
}

#[test]
fn check_queue_default_interval_without_limits() {
    // L1a: at exactly DEFAULT_CHECK since the last check, not one 1 ms before
    let at = Instant::now();
    assert!(!can_check_queue(None, None, 0, Some(at), at));
    assert!(!can_check_queue(
        None,
        None,
        0,
        Some(at),
        at + DEFAULT_CHECK - Duration::from_millis(1)
    ));
    assert!(can_check_queue(None, None, 0, Some(at), at + DEFAULT_CHECK));
}

#[test]
fn check_queue_timelimit_in_milliseconds() {
    // L1a: `timelimit: 5` is due at 5 s since the last check (1.0.8 and main: between 5
    // and 6 s, whole wall-clock seconds and a strict compare)
    let at = Instant::now();
    let five = Duration::from_secs(5);
    assert!(!can_check_queue(Some(5), None, 1, Some(at), at));
    assert!(!can_check_queue(
        Some(5),
        None,
        1,
        Some(at),
        at + five - Duration::from_millis(1)
    ));
    assert!(can_check_queue(Some(5), None, 1, Some(at), at + five));
}

#[test]
fn check_queue_clock_never_goes_back() {
    // An instant before the last check (cannot happen on a monotonic clock) is not due
    let at = Instant::now() + Duration::from_secs(10);
    assert!(!can_check_queue(Some(5), None, 1, Some(at), Instant::now()));
}

#[test]
fn check_queue_checklimit() {
    let at = Instant::now();
    assert!(can_check_queue(None, Some(100), 0, Some(at), at));
    assert!(!can_check_queue(None, Some(100), 3, Some(at), at));
    assert!(can_check_queue(Some(3600), Some(100), 0, Some(at), at));
}

#[test]
fn countdown_without_checklimit_never_underflows() {
    // A-C4, M1F-1: a target with limits and no checklimit, between two checks. Before,
    // every offer took one from a counter left at 0: a panic in a debug build, a wrap to
    // u64::MAX in a release build. No connection is used: no check is due.
    let c = config();
    let mut t = client("t1", "dst");
    t.hardlimit = Some(10);
    t.timelimit = Some(3600);
    let mut link = new_link(0, &c, &t, &None);
    link.last_check = Some(Instant::now());
    let mut deleted = 0;
    for _ in 0..3 {
        assert_eq!(can_send(0, &mut link, &mut deleted), Ok(true));
    }
    assert_eq!(link.packages, 0);
}

#[test]
fn countdown_with_checklimit_counts_down_to_zero() {
    let c = config();
    let mut t = client("t1", "dst");
    t.hardlimit = Some(10);
    t.checklimit = Some(2);
    t.timelimit = Some(3600);
    let mut link = new_link(0, &c, &t, &None);
    link.last_check = Some(Instant::now());
    link.packages = 2;
    let mut deleted = 0;
    assert_eq!(can_send(0, &mut link, &mut deleted), Ok(true));
    assert_eq!(link.packages, 1);
    assert_eq!(can_send(0, &mut link, &mut deleted), Ok(true));
    assert_eq!(link.packages, 0);
}

#[test]
fn zero_limits_read_as_absent() {
    // L4c-1, M3F-17: accepted as "no limits" by the check, and now delivered so too
    let mut c = config();
    c.clients[0].timelimit = Some(0);
    c.clients[0].checklimit = Some(0);
    c.clients[0].softlimit = Some(0);
    c.clients[0].hardlimit = Some(0);
    let c = verify_config(c).expect("all limits at 0 is a target without limits");
    let t = &c.clients[0];
    assert_eq!(
        (t.timelimit, t.checklimit, t.softlimit, t.hardlimit),
        (None, None, None, None)
    );
}

// --- verify_config ---

#[test]
fn verify_reads_deleteblock_zero_as_not_set() {
    // C4c-1 = a: a 1.0.8 file with it started; read as not set, the target is stuck at
    // hardlimit instead of the endless LTRIM loop (M3F-4), and a warning says so
    let mut c = config();
    c.clients[0].timelimit = Some(1);
    c.clients[0].checklimit = Some(1);
    c.clients[0].softlimit = Some(5);
    c.clients[0].hardlimit = Some(10);
    c.clients[0].deleteblock = Some(0);
    let warnings = config_warnings(&c);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("deleteblock: 0, read as not set")),
        "{:?}",
        warnings
    );
    let c = verify_config(c).expect("deleteblock 0 starts");
    assert_eq!(c.clients[0].deleteblock, None);
    assert_eq!(c.clients[0].hardlimit, Some(10));
}

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
    // M3R-10: the error names the keys found
    let mut c = config();
    c.filter_until = Some("#".into());
    c.filter_limit = Some(4);
    assert!(err(c).contains("Source 'source' sets filter_until, filter_limit without filter"));
    let mut c = config();
    c.clients[0].filter_replace = Some("x".into());
    assert!(err(c).contains("Client 't1' sets filter_replace without filter"));
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
fn verify_accepts_every_ordering_combination() {
    // D4d-2 = a: no combination of the three keys is an error; the odd ones warn
    let regex = Some("(?P<ts>\\d+)".to_string());
    for o in [None, Some(String::new()), regex.clone()] {
        for b in [None, Some(0), Some(1)] {
            for l in [None, Some(0), Some(64)] {
                let mut c = config();
                (c.ordering, c.ordering_buffer_time, c.ordering_limit) = (o.clone(), b, l);
                let warned = |text: &str| config_warnings(&c).iter().any(|w| w.contains(text));
                let without = warned("without ordering: ignored");
                let empty = warned("has an empty ordering");
                let no_time = warned("without ordering_buffer_time: packages are released at once, so only those that arrive together are sorted");
                let zero = warned("has ordering_buffer_time: 0, read as 1 second");
                assert_eq!(
                    without,
                    o.is_none() && (b.is_some() || l.is_some()),
                    "{:?}",
                    (&o, b, l)
                );
                assert_eq!(empty, o.as_deref() == Some(""), "{:?}", (&o, b, l));
                assert_eq!(no_time, o == regex && b.is_none(), "{:?}", (&o, b, l));
                assert_eq!(zero, o == regex && b == Some(0), "{:?}", (&o, b, l));
                assert!(!warned("still in order"), "{:?}", (&o, b, l));
                assert!(verify_config(c).is_ok(), "{:?}", (&o, b, l));
            }
        }
    }
}

#[test]
fn ordering_buffer_time_zero_holds_a_second() {
    // M3R-35 = R35a: 1.0.8 compared whole seconds, so 0 held a package up to 1 s
    assert_eq!(ordering_hold(None), Duration::ZERO);
    assert_eq!(ordering_hold(Some(0)), Duration::from_secs(1));
    assert_eq!(ordering_hold(Some(1)), Duration::from_secs(1));
    assert_eq!(ordering_hold(Some(30)), Duration::from_secs(30));
    assert_eq!(ordering_hold(Some(u64::MAX)), Duration::from_secs(u64::MAX));
}

#[test]
fn verify_ordering_limit_is_optional() {
    // M1F-5: ordering and ordering_buffer_time are enough, with no warning
    let mut c = config();
    c.ordering = Some("(?P<ts>\\d+)".into());
    c.ordering_buffer_time = Some(1);
    assert!(config_warnings(&c).is_empty());
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_rejects_an_ordering_regex_that_does_not_compile() {
    let mut c = config();
    c.ordering = Some("(".into());
    c.ordering_buffer_time = Some(1);
    assert!(err(c).contains("has an ordering Regex '(' that doesn't compile"));
}

#[test]
fn verify_limits_hardlimit_enables_defaults() {
    // M3-f = fa: a hardlimit alone turns the limits on; softlimit 80 % of it, at least 1
    // (L4c-2); timelimit 1 s and checklimit 10000, each when not given (L4c-5)
    let limits = |hard: u64, soft: Option<u64>, time: Option<u64>, check: Option<u64>| {
        let mut c = config();
        c.clients[0].hardlimit = Some(hard);
        c.clients[0].softlimit = soft;
        c.clients[0].timelimit = time;
        c.clients[0].checklimit = check;
        let t = verify_config(c).expect("a hardlimit is enough").clients[0].clone();
        (t.softlimit, t.timelimit, t.checklimit)
    };
    assert_eq!(
        limits(10, None, None, None),
        (Some(8), Some(1), Some(10000))
    );
    assert_eq!(limits(7, None, None, None).0, Some(5));
    assert_eq!(limits(1, None, None, None).0, Some(1));
    assert_eq!(limits(u64::MAX, None, None, None).0, Some(u64::MAX / 5 * 4));
    assert_eq!(
        limits(10, Some(3), None, None),
        (Some(3), Some(1), Some(10000))
    );
    assert_eq!(
        limits(10, None, Some(60), None),
        (Some(8), Some(60), Some(10000))
    );
    assert_eq!(
        limits(10, None, None, Some(50)),
        (Some(8), Some(1), Some(50))
    );
    // A file valid for 1.0.8 (all four) keeps its values
    assert_eq!(
        limits(10, Some(5), Some(1), Some(1)),
        (Some(5), Some(1), Some(1))
    );
}

/// The limits keys over the grid of the differential run against 1.0.8 (harness
/// `diff-limits`, outside the repository): which files start, and what the delivery reads
#[test]
fn limits_keys_accepted_set_over_the_grid() {
    const VALUES: [Option<u64>; 7] = [
        None,
        Some(0),
        Some(1),
        Some(3),
        Some(10),
        Some(5000),
        Some(u64::MAX),
    ];
    const BLOCKS: [Option<u64>; 4] = [None, Some(0), Some(1), Some(4)];
    let set = |v: Option<u64>| v.filter(|&x| x > 0);
    let mut accepted = 0;
    for t in VALUES {
        for k in VALUES {
            for s in VALUES {
                for h in VALUES {
                    for d in BLOCKS {
                        let mut c = config();
                        let target = &mut c.clients[0];
                        (target.timelimit, target.checklimit, target.softlimit) = (t, k, s);
                        (target.hardlimit, target.deleteblock) = (h, d);
                        let result = verify_config(c);
                        // Without a hardlimit no other limit key may be set (0 is not set)
                        let starts = set(h).is_some()
                            || (set(t).is_none() && set(k).is_none() && set(s).is_none());
                        let case = (t, k, s, h, d);
                        assert_eq!(result.is_ok(), starts, "{:?}", case);
                        if let Ok(c) = result {
                            accepted += 1;
                            let r = &c.clients[0];
                            // 0 read as absent; with a hardlimit the defaults fill the rest
                            let want = match set(h) {
                                None => (set(t), set(k), set(s), None),
                                Some(hard) => (
                                    Some(set(t).unwrap_or(1)),
                                    Some(set(k).unwrap_or(10000)),
                                    Some(set(s).unwrap_or(std::cmp::max(
                                        1,
                                        hard / 5 * 4 + hard % 5 * 4 / 5,
                                    ))),
                                    Some(hard),
                                ),
                            };
                            assert_eq!(
                                (r.timelimit, r.checklimit, r.softlimit, r.hardlimit),
                                want,
                                "{:?}",
                                case
                            );
                            // deleteblock 0 is not set either (C4c-1 = a)
                            assert_eq!(r.deleteblock, d.filter(|&x| x > 0), "{:?}", case);
                        }
                    }
                }
            }
        }
    }
    assert_eq!(accepted, 6924);
}

#[test]
fn verify_limits_need_a_hardlimit() {
    for key in ["timelimit", "checklimit", "softlimit"] {
        let mut c = config();
        let t = &mut c.clients[0];
        match key {
            "timelimit" => t.timelimit = Some(1),
            "checklimit" => t.checklimit = Some(1),
            _ => t.softlimit = Some(5),
        }
        assert!(
            err(c).contains(&format!("sets {} without hardlimit", key)),
            "{}",
            key
        );
    }
    // At 0 they are not set, also with hardlimit at 0
    let mut c = config();
    c.clients[0].softlimit = Some(0);
    c.clients[0].hardlimit = Some(0);
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
fn verify_warns_on_empty_client_name() {
    // M1F-6: an empty name works; a warning says so
    let mut c = config();
    c.clients[0].name = String::new();
    assert!(config_warnings(&c)
        .iter()
        .any(|w| w.contains("A client has an empty name")));
    assert!(verify_config(c).is_ok());
}

// --- worker statistics ---

#[test]
fn each_worker_writes_only_its_own_stats_slot() {
    // S3-1: the counter slot is the one per-worker index the delivery path keeps; a worker
    // writing another worker's slot misreports counts and stuck targets. Deterministic.
    for n in [1u16, 2, 3, 8] {
        let stats = stats_slots(n, 2);
        for id in 0..n {
            let k = id as u64 + 1;
            worker_slot(&stats, id).add(&Counts {
                incoming: k,
                outgoing: 10 * k,
                dropped: 100 * k,
                deleted: 1000 * k,
                ..Counts::default()
            });
            worker_slot(&stats, id).stuck_targets[(id % 2) as usize].store(true, Ordering::Relaxed);
        }
        for (i, w) in stats.iter().enumerate() {
            let k = i as u64 + 1;
            let got = (
                w.incoming.load(Ordering::Relaxed),
                w.outgoing.load(Ordering::Relaxed),
                w.dropped.load(Ordering::Relaxed),
                w.deleted.load(Ordering::Relaxed),
            );
            assert_eq!(got, (k, 10 * k, 100 * k, 1000 * k), "slot {}", i);
            assert!(w.stuck_targets[i % 2].load(Ordering::Relaxed), "slot {}", i);
            assert!(
                !w.stuck_targets[1 - i % 2].load(Ordering::Relaxed),
                "slot {}",
                i
            );
        }
    }
}

#[test]
fn stats_are_summed_and_reset_per_interval() {
    let stats = stats_slots(3, 3);
    let c = |n: u64| Counts {
        incoming: n,
        outgoing: 2 * n,
        dropped: 3 * n,
        deleted: 4 * n,
        filtered: 5 * n,
        stuck: 6 * n,
        failed: 7 * n,
    };
    stats[0].add(&c(1));
    stats[2].add(&c(10));
    stats[1].stuck_targets[2].store(true, Ordering::Relaxed);
    stats[0].down_targets[1].store(true, Ordering::Relaxed);
    assert_eq!(take_counts(&stats), c(11));
    assert_eq!(take_counts(&stats), Counts::default());
    assert_eq!(stuck_clients(&stats, 3), vec![2]);
    assert_eq!(down_clients(&stats, 3), vec![1]);
    stats[1].stuck_targets[2].store(false, Ordering::Relaxed);
    stats[0].down_targets[1].store(false, Ordering::Relaxed);
    assert!(stuck_clients(&stats, 3).is_empty());
    assert!(down_clients(&stats, 3).is_empty());
}

#[test]
fn stats_slots_do_not_share_a_cache_line() {
    let stats = stats_slots(2, 1);
    let a = &stats[0] as *const WorkerStats as usize;
    let b = &stats[1] as *const WorkerStats as usize;
    assert_eq!(std::mem::align_of::<WorkerStats>(), 64);
    assert!(b - a >= 64);
}

// --- spreader ---

/// The 1.0.8 spreader: try clients[0], rotate the list by one after every attempt, stop at
/// the first success or after one full turn. Returns the original index that took it.
fn old_spread(order: &mut [usize], mut attempt: impl FnMut(usize) -> bool) -> Option<usize> {
    let n = order.len();
    for _ in 0..n {
        let i = order[0];
        let ok = attempt(i);
        for k in 0..n - 1 {
            order.swap(k, k + 1);
        }
        if ok {
            return Some(i);
        }
    }
    None
}

#[test]
fn spreader_takes_turns_and_skips_refusing_targets() {
    let mut cursor = 0;
    let all = |_: usize| true;
    let got: Vec<Option<usize>> = (0..4).map(|_| spread(&mut cursor, 3, all)).collect();
    assert_eq!(got, vec![Some(0), Some(1), Some(2), Some(0)]);
    let mut cursor = 0;
    let not_1 = |i: usize| i != 1;
    let got: Vec<Option<usize>> = (0..4).map(|_| spread(&mut cursor, 3, not_1)).collect();
    assert_eq!(got, vec![Some(0), Some(2), Some(0), Some(2)]);
    // Nobody takes it: one full turn, cursor unchanged
    let mut cursor = 2;
    let mut tried = Vec::new();
    assert_eq!(
        spread(&mut cursor, 3, |i| {
            tried.push(i);
            false
        }),
        None
    );
    assert_eq!((tried, cursor), (vec![2, 0, 1], 2));
}

#[test]
fn spreader_by_index_matches_the_old_rotation() {
    // Differential against the 1.0.8 rotation over generated accept patterns: same target
    // for every package, same attempts in the same order.
    let mut seed: u64 = 0x2545F4914F6CDD1D;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut cases = 0;
    for n in 1..=6usize {
        for _ in 0..500 {
            let mut order: Vec<usize> = (0..n).collect();
            let mut cursor = 0;
            for _ in 0..20 {
                let accept: Vec<bool> = (0..n).map(|_| next() % 3 != 0).collect();
                let (mut a, mut b) = (Vec::new(), Vec::new());
                let old = old_spread(&mut order, |i| {
                    a.push(i);
                    accept[i]
                });
                let new = spread(&mut cursor, n, |i| {
                    b.push(i);
                    accept[i]
                });
                assert_eq!((old, &a), (new, &b), "n={} accept={:?}", n, accept);
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 60000);
}

#[test]
fn pause_ends_early_on_stop() {
    let start = Instant::now();
    pause(|| true, Duration::from_secs(5));
    assert!(start.elapsed() < Duration::from_millis(50));
    let start = Instant::now();
    pause(|| false, Duration::from_millis(250));
    assert!(start.elapsed() >= Duration::from_millis(250));
    let stop = AtomicBool::new(false);
    let start = Instant::now();
    thread::scope(|s| {
        s.spawn(|| {
            thread::sleep(Duration::from_millis(150));
            stop.store(true, Ordering::Relaxed);
        });
        pause(|| stop.load(Ordering::Relaxed), Duration::from_secs(5));
    });
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn held_packages_compare_by_ts_then_arrival_order() {
    let now = Instant::now();
    let a = Held {
        ts: 1,
        seq: 0,
        arrival: now,
        data: b"a".to_vec(),
    };
    let b = Held {
        ts: 1,
        seq: 0,
        arrival: now + Duration::from_secs(1),
        data: b"b".to_vec(),
    };
    let c = Held {
        ts: 1,
        seq: 1,
        arrival: now,
        data: b"a".to_vec(),
    };
    assert!(a == b);
    assert!(a != c && a < c);
}

// --- request encoding (M3-o) ---

#[test]
fn rpush_is_packed_byte_for_byte_as_the_crate_packs_it() {
    let mut buf = Vec::new();
    let mut cases = 0;
    let keys: [&[u8]; 4] = [
        b"q",
        b"rmtest:a:b:t0",
        "clave-\u{e9}".as_bytes(),
        &[0, 255, b'\r', b'\n'],
    ];
    for key in keys {
        for len in [0usize, 1, 9, 10, 99, 100, 255, 1000, 65536] {
            let value: Vec<u8> = (0..len).map(|i| (i * 7 + len) as u8).collect();
            pack_rpush(&mut buf, key, &value);
            let want = redis::cmd("RPUSH")
                .arg(key)
                .arg(&value[..])
                .get_packed_command();
            assert_eq!(buf, want, "key {:?} len {}", key, len);
            cases += 1;
        }
    }
    // The buffer is reused: a shorter request after a longer one leaves nothing behind
    pack_rpush(&mut buf, b"k", b"v");
    assert_eq!(buf, b"*3\r\n$5\r\nRPUSH\r\n$1\r\nk\r\n$1\r\nv\r\n");
    assert_eq!(cases, 36);
}

/// The URL the program built before M3-i: `<scheme>://:<password>@<host>:<port>`
fn url_info(
    ssl: bool,
    host: &str,
    port: u16,
    password: &str,
) -> redis::RedisResult<redis::ConnectionInfo> {
    use redis::IntoConnectionInfo;
    let scheme = if ssl { "rediss" } else { "redis" };
    format!("{}://:{}@{}:{}", scheme, password, host, port).into_connection_info()
}

fn same_info(a: &redis::ConnectionInfo, b: &redis::ConnectionInfo) -> bool {
    a.addr == b.addr
        && a.redis.db == b.redis.db
        && a.redis.username == b.redis.username
        && a.redis.password == b.redis.password
        && a.redis.protocol == b.redis.protocol
}

#[test]
fn endpoint_reads_as_the_url_did_where_the_url_worked() {
    // Differential (M3-i): every host, port, TLS and password the URL path of 1.0.8 read as
    // written gives the same connection description; where the URL changed the password
    // (a percent sequence) or failed (a reserved character), the structured path keeps it
    // as written
    // Upper case too: the URL path kept a host as written (an opaque host of a scheme the
    // url crate does not know), and so does the structured path
    let hosts = [
        "127.0.0.1",
        "localhost",
        "redis.example.org",
        "10.0.0.7",
        "my-host_1",
        "LocalHost",
        "REDIS.Example.ORG",
    ];
    let passwords = [
        "",
        "secret",
        "p@ss",
        "with space",
        "a:b",
        "x;y",
        "semi,colon",
        "dollar$",
        "tilde~",
        "plus+",
        "eq=",
        "amp&",
        "excl!",
        "quote'",
        "star*",
        "paren()",
        "brack[]",
        "%41",
        "%2F",
        "100%",
        "#hash",
        "slash/",
        "ques?",
        "tab\t",
        "uni\u{e9}",
        "back\\",
    ];
    let mut same = 0;
    let mut differ = Vec::new();
    for ssl in [false, true] {
        for host in hosts {
            for port in [6379u16, 6390, 1, 65535] {
                for password in passwords {
                    let ep =
                        Endpoint::new(Some(ssl), host, port, &None, password, None, None, None);
                    match url_info(ssl, host, port, password) {
                        Ok(url) if url.redis.password.as_deref().unwrap_or("") == password => {
                            assert!(
                                same_info(&ep.info(), &url),
                                "differs: ssl={} host={} port={} password={:?}: {:?} against {:?}",
                                ssl,
                                host,
                                port,
                                password,
                                ep.info(),
                                url
                            );
                            same += 1;
                        }
                        other => {
                            // The structured path passes the password through untouched
                            assert_eq!(ep.info().redis.password.as_deref(), Some(password));
                            differ.push((password, other.is_ok()));
                        }
                    }
                }
            }
        }
    }
    assert!(same >= 1120, "only {} cases compared", same);
    let mut changed: Vec<&str> = differ.iter().map(|(p, _)| *p).collect();
    changed.sort();
    changed.dedup();
    // The passwords 1.0.8 could not use as written (M1F-7)
    assert_eq!(
        changed,
        vec!["#hash", "%2F", "%41", "ques?", "slash/", "tab\t"]
    );
}

#[test]
fn endpoint_database_and_username() {
    let ep = Endpoint::new(
        None,
        "h",
        1,
        &Some("app".to_string()),
        "pw",
        Some(3),
        None,
        None,
    );
    let info = ep.info();
    assert_eq!(info.redis.db, 3);
    assert_eq!(info.redis.username.as_deref(), Some("app"));
    assert_eq!(info.redis.password.as_deref(), Some("pw"));
    // Empty user and password mean none, as the URL path read them: no AUTH
    let ep = Endpoint::new(None, "h", 1, &Some(String::new()), "", None, None, None);
    assert_eq!(
        (ep.info().redis.username, ep.info().redis.password),
        (None, None)
    );
    assert_eq!(ep.info().redis.db, 0);
}

#[test]
fn yaml_database_and_username_are_optional() {
    let base = "name: s\nhostname: h\nport: 1\npassword: ''\nchannel: c\nchildren: 1\nmode: replicant\nclients:\n  - name: t\n    hostname: h\n    port: 1\n    password: ''\n    channel: d\n";
    let c = parse_yaml(base).unwrap();
    assert_eq!((c.username.clone(), c.database), (None, None));
    assert_eq!(
        (c.clients[0].username.clone(), c.clients[0].database),
        (None, None)
    );
    let with = base.replace("channel: c\n", "channel: c\nusername: u\ndatabase: 2\n")
        + "    username: v\n    database: 5\n";
    let c = parse_yaml(&with).unwrap();
    assert_eq!((c.username.as_deref(), c.database), (Some("u"), Some(2)));
    assert_eq!(
        (c.clients[0].username.as_deref(), c.clients[0].database),
        (Some("v"), Some(5))
    );
    // A database below 0 is not a database
    assert!(parse_yaml(&base.replace("channel: c\n", "channel: c\ndatabase: -1\n")).is_err());
}

#[test]
fn timeouts_default_zero_and_values() {
    let ep = |c: Option<f64>, m: Option<f64>| Endpoint::new(None, "h", 1, &None, "", None, c, m);
    // Absent: 2 s and 5 s, as since the stage 3 fix round
    let d = ep(None, None);
    assert_eq!(d.connect_timeout, Some(Duration::from_secs(2)));
    assert_eq!(d.command_timeout, Some(Duration::from_secs(5)));
    // 0: no limit, as 1.0.8 waited
    let z = ep(Some(0.0), Some(0.0));
    assert_eq!((z.connect_timeout, z.command_timeout), (None, None));
    // Decimals
    let v = ep(Some(0.25), Some(30.0));
    assert_eq!(v.connect_timeout, Some(Duration::from_millis(250)));
    assert_eq!(v.command_timeout, Some(Duration::from_secs(30)));
}

#[test]
fn verify_rejects_timeouts_that_are_not_durations() {
    for bad in [-1.0, f64::NAN, f64::INFINITY, 1e30] {
        let mut c = config();
        c.connect_timeout = Some(bad);
        assert!(err(c).contains("invalid connect_timeout"), "{}", bad);
        let mut c = config();
        c.clients[0].command_timeout = Some(bad);
        assert!(err(c).contains("invalid command_timeout"), "{}", bad);
    }
    let mut c = config();
    c.connect_timeout = Some(0.0);
    c.clients[0].command_timeout = Some(0.5);
    assert!(verify_config(c).is_ok());
}

#[test]
fn verify_rejects_timeouts_below_a_millisecond() {
    // M3R-27: 1e-10 became a zero Duration, refused by the socket at every attempt
    for bad in [1e-10, 0.0001, 0.000999] {
        let mut c = config();
        c.clients[0].connect_timeout = Some(bad);
        assert!(err(c).contains("at least 0.001 seconds"), "{}", bad);
        let mut c = config();
        c.command_timeout = Some(bad);
        assert!(err(c).contains("invalid command_timeout"), "{}", bad);
    }
    let mut c = config();
    c.connect_timeout = Some(0.001);
    c.clients[0].command_timeout = Some(0.001);
    assert!(verify_config(c).is_ok());
}

#[test]
fn yaml_timeouts_accept_integers_and_decimals() {
    let base = "name: s\nhostname: h\nport: 1\npassword: ''\nchannel: c\nchildren: 1\nmode: replicant\nclients:\n  - name: t\n    hostname: h\n    port: 1\n    password: ''\n    channel: d\n";
    let c = parse_yaml(base).unwrap();
    assert_eq!((c.connect_timeout, c.command_timeout), (None, None));
    let with = base.replace(
        "channel: c\n",
        "channel: c\nconnect_timeout: 3\ncommand_timeout: 0\n",
    ) + "    connect_timeout: 0.2\n    command_timeout: 1.5\n";
    let c = parse_yaml(&with).unwrap();
    assert_eq!(
        (c.connect_timeout, c.command_timeout),
        (Some(3.0), Some(0.0))
    );
    assert_eq!(
        (c.clients[0].connect_timeout, c.clients[0].command_timeout),
        (Some(0.2), Some(1.5))
    );
}

#[test]
fn config_warnings_for_duplicate_names_and_soft_above_hard() {
    let mut c = config();
    c.clients.push(client("t1", "dst2"));
    c.clients[0].hardlimit = Some(10);
    c.clients[0].softlimit = Some(10);
    let w = config_warnings(&c);
    assert!(
        w.iter()
            .any(|w| w.contains("Client 't1' is the name of more than one client")),
        "{:?}",
        w
    );
    assert!(
        w.iter()
            .any(|w| w.contains("softlimit 10 equal to hardlimit: the target is freed only when its list goes under hardlimit")),
        "{:?}",
        w
    );
    assert!(config_warnings(&config()).is_empty());
}

#[test]
fn limit_warnings_say_what_happens() {
    // M3R-38: softlimit above hardlimit frees the target at the next check; equal to it,
    // only under hardlimit; deleteblock 0 without hardlimit has nothing to be stuck at
    let warned = |soft: Option<u64>, hard: Option<u64>, block: Option<u64>| {
        let mut c = config();
        (
            c.clients[0].softlimit,
            c.clients[0].hardlimit,
            c.clients[0].deleteblock,
        ) = (soft, hard, block);
        config_warnings(&c)
    };
    let w = warned(Some(12), Some(10), None);
    assert!(
        w.iter().any(|w| w.contains(
            "softlimit 12 above hardlimit 10: the target is freed at the check right after it gets stuck"
        )),
        "{:?}",
        w
    );
    let w = warned(Some(10), Some(10), None);
    assert!(
        w.iter()
            .any(|w| w.contains("freed only when its list goes under hardlimit")),
        "{:?}",
        w
    );
    assert!(!w.iter().any(|w| w.contains("right after")), "{:?}", w);
    assert!(warned(Some(9), Some(10), None).is_empty());
    let w = warned(None, Some(10), Some(0));
    assert!(
        w.iter()
            .any(|w| w.contains("stuck at hardlimit instead of trimmed")),
        "{:?}",
        w
    );
    let w = warned(None, None, Some(0));
    assert_eq!(
        w,
        vec!["Client 't1' has deleteblock: 0, read as not set".to_string()]
    );
}

#[test]
fn unknown_keys_are_named() {
    let yaml = "name: s
hostname: h
port: 1
password: ''
channel: c
children: 1
mode: replicant
colour: blue
hardlimit: 10
clients:
  - name: t0
    hostname: h
    port: 1
    password: ''
    channel: d
    filter_untill: '#'
    hardlimit: 10
  - name: 7
    hostname: h
    port: 1
    password: ''
    channel: e
    x: 1
";
    assert!(parse_yaml(yaml).is_ok());
    assert_eq!(
        unknown_keys(yaml),
        vec![
            "Unknown key 'colour' at the root of the configuration: ignored".to_string(),
            // A limits key is a client's: at the root it is unknown (M3R-39)
            "Unknown key 'hardlimit' at the root of the configuration: ignored".to_string(),
            "Unknown key 'filter_untill' in client 't0': ignored".to_string(),
            "Unknown key 'x' in client '#2': ignored".to_string(),
        ]
    );
    // An unquoted number on a text key reads as text (as 1.0.8) and is not a warning
    let yaml = yaml.replace("password: ''\nchannel: c", "password: 1234\nchannel: c");
    assert_eq!(parse_yaml(&yaml).unwrap().password, "1234");
    assert_eq!(unknown_keys(&yaml).len(), 4);
    // Text the untyped read cannot take: no warnings, nothing else
    assert!(unknown_keys("[1, 2").is_empty());
}

#[test]
fn known_keys_are_the_configuration_fields() {
    // Every field of Config and ClientConfig, and nothing else, as the key lists say
    let mut c = config();
    c.ssl = Some(true);
    let fields = |v: serde_norway::Value| -> Vec<String> {
        match v {
            serde_norway::Value::Mapping(m) => {
                m.keys().map(|k| k.as_str().unwrap().to_string()).collect()
            }
            _ => panic!("not a mapping"),
        }
    };
    let mut root = fields(serde_norway::to_value(&c).unwrap());
    let mut client = fields(serde_norway::to_value(&c.clients[0]).unwrap());
    root.sort();
    client.sort();
    let mut want_root: Vec<String> = ROOT_KEYS.iter().map(|k| k.to_string()).collect();
    let mut want_client: Vec<String> = CLIENT_KEYS.iter().map(|k| k.to_string()).collect();
    want_root.sort();
    want_client.sort();
    assert_eq!(root, want_root);
    assert_eq!(client, want_client);
}

#[test]
fn ipv6_host_with_or_without_brackets() {
    // M3R-28: both forms reach the resolver as the address
    for host in ["[::1]", "::1"] {
        let mut t = client("t", "c");
        t.hostname = host.into();
        match Endpoint::target(&t).info().addr {
            redis::ConnectionAddr::Tcp(h, _) => assert_eq!(h, "::1", "{}", host),
            other => panic!("{:?}", other),
        }
    }
    // Only one pair, and only around the whole host
    assert_eq!(unbracketed("[[::1]]"), "[::1]");
    assert_eq!(unbracketed("[::1"), "[::1");
    assert_eq!(unbracketed("redis.example.org"), "redis.example.org");
    let mut c = config();
    c.hostname = "[2001:db8::1]".into();
    c.ssl = Some(true);
    match Endpoint::source(&c).info().addr {
        redis::ConnectionAddr::TcpTls { host, .. } => assert_eq!(host, "2001:db8::1"),
        other => panic!("{:?}", other),
    }
}

// --- supervise: a thread that dies (S4-5 = a, D4d-6 = a) ---

/// `panic_after` of a test worker that panics when it sees the stop
const PANIC_AT_STOP: u32 = u32::MAX;

/// A worker for supervise's tests: pushes one package every millisecond until the stop,
/// or panics after `panic_after` pushes (holding the buffer's lock when `holding`), or at
/// the stop (PANIC_AT_STOP)
fn test_worker(
    stop: Arc<AtomicBool>,
    buffer: Arc<OrderBuffer>,
    pushed: Arc<AtomicU64>,
    panic_after: Option<u32>,
    holding: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut n = 0u32;
        while !stop.load(Ordering::Relaxed) {
            if panic_after == Some(n) {
                if holding {
                    let _held = buffer.lock();
                    panic!("test worker dies holding the lock");
                }
                panic!("test worker dies");
            }
            buffer.push(n as u128, n.to_be_bytes().to_vec(), Instant::now());
            pushed.fetch_add(1, Ordering::Relaxed);
            n += 1;
            thread::sleep(Duration::from_millis(1));
        }
        if panic_after == Some(PANIC_AT_STOP) {
            panic!("test worker dies at the stop");
        }
    })
}

/// A sender for supervise's tests: the real send_ordered, counting what it delivers;
/// panics at its first batch when `dies`
fn test_sender(
    buffer: Arc<OrderBuffer>,
    delivered: Arc<AtomicU64>,
    dies: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        send_ordered(&buffer, Duration::ZERO, |turn| {
            if let Some(batch) = turn {
                if dies {
                    panic!("test sender dies");
                }
                delivered.fetch_add(batch.len() as u64, Ordering::Relaxed);
            }
        })
    })
}

/// Runs supervise over two workers and a sender; returns the ending, what was pushed and
/// what was delivered. A signal is sent after `signal_after`, or after 6 s as a backstop:
/// a thread that dies is seen within a second, so a run that ends by the backstop is a
/// death not seen (died false), not a test that waits for ever.
fn supervised(
    panic_after: [Option<u32>; 2],
    holding: bool,
    sender_dies: bool,
    signal_after: Option<Duration>,
) -> (Ending, u64, u64) {
    let stop = Arc::new(AtomicBool::new(false));
    let buffer = Arc::new(OrderBuffer::new());
    let delivered = Arc::new(AtomicU64::new(0));
    let pushed = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel();
    let workers: Vec<thread::JoinHandle<()>> = panic_after
        .iter()
        .map(|&p| test_worker(stop.clone(), buffer.clone(), pushed.clone(), p, holding))
        .collect();
    let sender = test_sender(buffer.clone(), delivered.clone(), sender_dies);
    let after = signal_after.unwrap_or(Duration::from_secs(6));
    thread::spawn(move || {
        thread::sleep(after);
        let _ = tx.send(false);
    });
    let start = Instant::now();
    let ending = supervise(&stop, &rx, workers, Some((buffer.clone(), sender)), || {});
    // A thread that dies on its own is seen within main's one-second wait, well before the
    // backstop
    let dies = sender_dies
        || panic_after
            .iter()
            .any(|p| p.is_some_and(|n| n != PANIC_AT_STOP));
    let bound = if dies { 4 } else { 10 };
    assert!(
        start.elapsed() < Duration::from_secs(bound),
        "supervise took {:?}",
        start.elapsed()
    );
    assert_eq!(buffer.len(), 0, "packages left in the buffer");
    (
        ending,
        pushed.load(Ordering::Relaxed),
        delivered.load(Ordering::Relaxed),
    )
}

#[test]
fn supervise_signal_is_a_clean_stop() {
    let (ending, pushed, delivered) =
        supervised([None, None], false, false, Some(Duration::from_millis(300)));
    assert_eq!(
        ending,
        Ending {
            died: false,
            abandoned: 0
        }
    );
    assert_eq!(delivered, pushed);
}

#[test]
fn supervise_a_stop_flag_without_the_wake_up_is_a_clean_stop() {
    // M3R-34: the handler stores the stop flag, then prints, then wakes main. Here the
    // flag is set, the workers leave at once and nothing comes on the channel: a clean
    // stop, not a death, and seen within main's one-second turn
    let stop = Arc::new(AtomicBool::new(false));
    let buffer = Arc::new(OrderBuffer::new());
    let delivered = Arc::new(AtomicU64::new(0));
    let pushed = Arc::new(AtomicU64::new(0));
    let (_tx, rx) = mpsc::channel::<bool>();
    let workers: Vec<thread::JoinHandle<()>> = (0..2)
        .map(|_| test_worker(stop.clone(), buffer.clone(), pushed.clone(), None, false))
        .collect();
    let sender = test_sender(buffer.clone(), delivered.clone(), false);
    thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::Relaxed);
    // The workers have left before main's next turn, as a busy worker does
    let deadline = Instant::now() + Duration::from_secs(2);
    while !workers.iter().all(|h| h.is_finished()) {
        assert!(Instant::now() < deadline, "the test workers did not leave");
        thread::sleep(Duration::from_millis(10));
    }
    let start = Instant::now();
    let ending = supervise(&stop, &rx, workers, Some((buffer.clone(), sender)), || {});
    assert_eq!(
        ending,
        Ending {
            died: false,
            abandoned: 0
        }
    );
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        delivered.load(Ordering::Relaxed),
        pushed.load(Ordering::Relaxed)
    );
}

#[test]
fn supervise_a_worker_that_dies_stops_the_program() {
    // The other worker is told to stop, the sender flushes everything: nothing lost
    let (ending, pushed, delivered) = supervised([Some(50), None], false, false, None);
    assert_eq!(
        ending,
        Ending {
            died: true,
            abandoned: 0
        }
    );
    assert!(delivered > 0);
    assert_eq!(delivered, pushed);
}

#[test]
fn supervise_a_worker_that_dies_at_the_stop() {
    // A panic while the program stops is a death too: exit 1, nothing lost
    let (ending, pushed, delivered) = supervised(
        [Some(PANIC_AT_STOP), None],
        false,
        false,
        Some(Duration::from_millis(300)),
    );
    assert_eq!(
        ending,
        Ending {
            died: true,
            abandoned: 0
        }
    );
    assert_eq!(delivered, pushed);
}

#[test]
fn supervise_a_worker_that_dies_holding_the_buffer() {
    // The lock is poisoned: the other worker and the sender go on with it
    let (ending, pushed, delivered) = supervised([Some(50), None], true, false, None);
    assert_eq!(
        ending,
        Ending {
            died: true,
            abandoned: 0
        }
    );
    assert_eq!(delivered, pushed);
}

#[test]
fn supervise_a_sender_that_dies_leaves_the_buffer_counted() {
    // The workers are stopped and joined; what the buffer holds is abandoned, counted, and
    // so is the batch the sender had taken and not counted when it died while running
    // (M3R-41): every package pushed is abandoned
    let (ending, pushed, delivered) = supervised([None, None], false, true, None);
    assert!(ending.died);
    assert_eq!(delivered, 0);
    assert!(pushed > 0);
    assert_eq!(ending.abandoned, pushed, "{:?} of {}", ending, pushed);
}

#[test]
fn a_sender_that_dies_in_the_stop_flush_leaves_what_it_held_counted() {
    // M3R-41: at the stop finish() releases everything, so the sender's batch is the whole
    // buffer. It counts chunk by chunk; a death after the first chunk leaves the other two
    // chunks counted as abandoned, not lost from every count
    let buffer = Arc::new(OrderBuffer::new());
    for n in 0..3000u32 {
        buffer.push(u128::from(n), n.to_be_bytes().to_vec(), Instant::now());
    }
    buffer.finish();
    let delivered = Arc::new(AtomicU64::new(0));
    let (b, d) = (buffer.clone(), delivered.clone());
    let sender = thread::spawn(move || {
        send_ordered(&b, Duration::from_secs(3600), |turn| {
            if let Some(batch) = turn {
                for chunk in chunks(&batch, CHUNK_PACKAGES) {
                    if d.load(Ordering::Relaxed) > 0 {
                        panic!("test sender dies in the flush");
                    }
                    d.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    b.counted(chunk.len());
                }
            }
        })
    });
    assert!(sender.join().is_err());
    assert_eq!(delivered.load(Ordering::Relaxed), 1000);
    assert_eq!(buffer.abandon(), 2000);
    assert_eq!(buffer.abandon(), 0);
}

#[test]
fn supervise_a_sender_that_dies_holding_the_buffer() {
    // A thread that dies holding the lock, then the sender: nothing waits for ever
    let stop = Arc::new(AtomicBool::new(false));
    let buffer = Arc::new(OrderBuffer::new());
    let b = buffer.clone();
    let _ = thread::spawn(move || {
        let _held = b.lock();
        panic!("dies holding the lock");
    })
    .join();
    buffer.push(1, vec![1], Instant::now());
    assert_eq!(buffer.len(), 1);
    let delivered = Arc::new(AtomicU64::new(0));
    let sender = test_sender(buffer.clone(), delivered.clone(), true);
    let worker = test_worker(
        stop.clone(),
        buffer.clone(),
        Arc::new(AtomicU64::new(0)),
        None,
        false,
    );
    let (tx, rx) = mpsc::channel::<bool>();
    // The backstop of supervised()
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(6));
        let _ = tx.send(false);
    });
    let ending = supervise(
        &stop,
        &rx,
        vec![worker],
        Some((buffer.clone(), sender)),
        || {},
    );
    assert!(ending.died);
    assert!(ending.abandoned >= 1);
    assert_eq!(buffer.len(), 0);
}

// --- the status file (M3-e = ea, D4d-4 = a) ---

/// A directory of its own for one test
fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmunit-{}-{}", test, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn status_file_is_replaced_whole() {
    use std::io::Read;
    let dir = scratch_dir("status_whole");
    let path = dir.join("rm.status");
    write_status(&path, b"{\"first\":1}").unwrap();
    // A reader that opened the file before the next write still reads the old text whole:
    // the new text is another file renamed over this one, not written into it
    let mut before = fs::File::open(&path).unwrap();
    write_status(&path, b"{\"second\":2}").unwrap();
    let mut old = String::new();
    before.read_to_string(&mut old).unwrap();
    assert_eq!(old, "{\"first\":1}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"second\":2}");
    assert!(!dir.join("rm.status.tmp").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_file_keeps_its_mode_and_group() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = scratch_dir("status_mode");
    let path = dir.join("rm.status");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    // Another group of this user, if it has one (a group it may give the file)
    let mut groups = vec![0 as libc::gid_t; 64];
    // SAFETY: getgroups writes at most `len` ids into the buffer
    let n = unsafe { libc::getgroups(groups.len() as i32, groups.as_mut_ptr()) };
    let own = fs::metadata(&path).unwrap().gid();
    let other = groups[..n.max(0) as usize]
        .iter()
        .copied()
        .find(|&g| g != own);
    if let Some(g) = other {
        std::os::unix::fs::chown(&path, None, Some(g)).unwrap();
    }
    let want = fs::metadata(&path).unwrap();
    write_status(&path, b"new").unwrap();
    let got = fs::metadata(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(got.mode() & 0o7777, 0o640);
    assert_eq!((got.uid(), got.gid()), (want.uid(), want.gid()));
    assert_ne!(got.ino(), want.ino(), "not replaced");
    if other.is_none() {
        eprintln!("status_file_keeps_its_mode_and_group: one group only, mode checked alone");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_file_through_a_symbolic_link() {
    let dir = scratch_dir("status_link");
    fs::create_dir_all(dir.join("real")).unwrap();
    let real = dir.join("real").join("rm.status");
    fs::write(&real, "old").unwrap();
    let link = dir.join("rm.status");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write_status(&link, b"new").unwrap();
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), real);
    assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    // The temporary file went beside the target, and is gone
    assert!(!dir.join("real").join("rm.status.tmp").exists());
    assert!(!dir.join("rm.status.tmp").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_file_never_written_through_what_is_at_the_temporary_name() {
    // M3R-40: a stale file at <status>.tmp is replaced, not reused; a link planted there
    // is removed, and the file it points to is left as it was
    use std::os::unix::fs::MetadataExt;
    let dir = scratch_dir("status_tmp_name");
    let path = dir.join("rm.status");
    let tmp = dir.join("rm.status.tmp");
    fs::write(&path, "old").unwrap();
    let before = fs::metadata(&path).unwrap().ino();
    fs::write(&tmp, "stale stale stale stale").unwrap();
    write_status(&path, b"new").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    // Replaced whole, not written in place for want of the temporary file
    assert_ne!(fs::metadata(&path).unwrap().ino(), before);
    assert!(!tmp.exists());
    let victim = dir.join("victim");
    fs::write(&victim, "keep me").unwrap();
    std::os::unix::fs::symlink(&victim, &tmp).unwrap();
    write_status(&path, b"newer").unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
    assert_eq!(fs::read_to_string(&path).unwrap(), "newer");
    assert!(!fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(&tmp).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_file_written_in_place_when_it_cannot_be_replaced() {
    // Where the temporary file cannot be made (here a directory has its name; in use, a
    // directory the user may not write), the file is written in place, as before
    let dir = scratch_dir("status_in_place");
    let path = dir.join("rm.status");
    fs::write(&path, "old").unwrap();
    fs::create_dir(dir.join("rm.status.tmp")).unwrap();
    write_status(&path, b"new").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert!(STATUS_IN_PLACE.load(Ordering::Relaxed));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_temporary_file_is_removed_when_the_replace_fails() {
    // M3R-39: the temporary file is made and the rename fails (the status path is a
    // directory): the error is returned and no `<status>.tmp` is left behind
    let dir = scratch_dir("status_tmp_left");
    let path = dir.join("rm.status");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("inside"), "x").unwrap();
    assert!(write_replacing(&path, b"new").is_err());
    assert!(fs::symlink_metadata(dir.join("rm.status.tmp")).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_reachable_target_connects_inline() {
    // M3R-26: a target without a link whose retry is due gets a connect attempt that the
    // delivering thread waits for up to CONNECT_GRACE, so a target that answers takes the
    // package that found it down (without the grace, that package skipped it)
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        // A stand-in that answers +OK to whatever comes (CLIENT SETNAME at the connect)
        for mut stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 || stream.write_all(b"+OK\r\n").is_err() {
                        break;
                    }
                }
            });
        }
    });
    let mut c = config();
    c.clients[0].hostname = "127.0.0.1".into();
    c.clients[0].port = port;
    let mut target = new_link(0, &c, &c.clients[0], &None);
    assert!(target.link.is_none());
    assert!(ensure_link(0, &mut target), "not connected inline");
    assert!(target.link.is_some());
}

#[test]
fn chunks_hold_at_most_the_bound() {
    // P3 = p3a: 1000 packages or 4 MiB of them, whichever comes first; a larger package alone
    let small: Vec<Vec<u8>> = (0..2500).map(|i| vec![b'x'; 10 + i % 7]).collect();
    let sizes: Vec<usize> = chunks(&small, CHUNK_PACKAGES)
        .iter()
        .map(|c| c.len())
        .collect();
    assert_eq!(sizes, vec![1000, 1000, 500]);
    let mib = vec![vec![b'y'; 1 << 20]; 9];
    let sizes: Vec<usize> = chunks(&mib, CHUNK_PACKAGES)
        .iter()
        .map(|c| c.len())
        .collect();
    assert_eq!(sizes, vec![4, 4, 1]);
    let mixed = vec![vec![1u8; 10], vec![2u8; 5 << 20], vec![3u8; 10]];
    let cut = chunks(&mixed, CHUNK_PACKAGES);
    assert_eq!(
        cut.iter().map(|c| c.len()).collect::<Vec<_>>(),
        vec![1, 1, 1]
    );
    assert_eq!(cut[1][0].len(), 5 << 20);
    assert!(chunks(&[], CHUNK_PACKAGES).is_empty());
    // Every package once, in order
    let joined: Vec<&Vec<u8>> = chunks(&small, CHUNK_PACKAGES)
        .into_iter()
        .flatten()
        .collect();
    assert!(joined.iter().zip(&small).all(|(a, b)| *a == b) && joined.len() == small.len());
}

#[test]
fn a_package_counts_once_with_one_reason() {
    // D4d-3 = a: failed before stuck before filtered; a taken package is outgoing
    let cases = [
        (true, false, false, "outgoing"),
        (true, true, true, "outgoing"),
        (false, true, true, "failed"),
        (false, false, true, "failed"),
        (false, true, false, "stuck"),
        (false, false, false, "filtered"),
    ];
    for (delivered, stuck, failed, want) in cases {
        let mut c = Counts::default();
        account(delivered, &Why { stuck, failed }, &mut c);
        let got = [
            ("outgoing", c.outgoing),
            ("failed", c.failed),
            ("stuck", c.stuck),
            ("filtered", c.filtered),
        ];
        let named: Vec<&str> = got
            .iter()
            .filter(|(_, n)| *n == 1)
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(
            named,
            vec![want],
            "delivered {delivered} stuck {stuck} failed {failed}"
        );
        assert_eq!(c.dropped, u64::from(!delivered));
    }
}

/// A stand-in target for the sender's chunks: +OK to anything but RPUSH; the RPUSHes are
/// answered by `script` in turn (Some(reply) as given, None: no reply, and none after it)
fn scripted_target(script: Vec<Option<&'static str>>) -> u16 {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let Some(Ok(stream)) = listener.incoming().next() else {
            return;
        };
        let mut w = stream.try_clone().unwrap();
        let mut r = BufReader::new(stream);
        let mut script = script.into_iter();
        let mut mute = false;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let n: usize = line.trim_start_matches('*').trim().parse().unwrap();
            let mut args = Vec::new();
            for _ in 0..n {
                line.clear();
                r.read_line(&mut line).unwrap();
                let len: usize = line.trim_start_matches('$').trim().parse().unwrap();
                let mut buf = vec![0; len + 2];
                r.read_exact(&mut buf).unwrap();
                buf.truncate(len);
                args.push(buf);
            }
            let reply = if args[0].eq_ignore_ascii_case(b"RPUSH") {
                match script.next().flatten() {
                    Some(reply) if !mute => reply.to_string(),
                    _ => {
                        mute = true;
                        continue;
                    }
                }
            } else {
                "+OK\r\n".to_string()
            };
            if w.write_all(reply.as_bytes()).is_err() {
                return;
            }
        }
    });
    port
}

#[test]
fn flush_target_reads_reply_by_reply_and_stops_at_a_broken_link() {
    // Plan 9.2/9.3: an integer reply is taken, an error reply refused (that RPUSH only, the
    // link kept), the first reply that does not come makes it and the rest unknown and
    // drops the link
    let port = scripted_target(vec![
        Some(":1\r\n"),
        Some("-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
        Some(":2\r\n"),
        None,
    ]);
    let mut c = config();
    c.clients[0].hostname = "127.0.0.1".into();
    c.clients[0].port = port;
    c.clients[0].command_timeout = Some(0.3);
    let mut target = new_link(0, &c, &c.clients[0], &None);
    assert!(ensure_link(0, &mut target));
    for p in 0..5 {
        append_rpush(&mut target.request, b"dst", format!("p{p}").as_bytes());
        target.pending.push(p);
    }
    let t = Instant::now();
    let (replies, error) = flush_target(&mut target);
    assert_eq!(
        replies,
        vec![
            (0, Reply::Taken),
            (1, Reply::Refused),
            (2, Reply::Taken),
            (3, Reply::Unknown),
            (4, Reply::Unknown)
        ]
    );
    assert!(error.unwrap().contains("WRONGTYPE"));
    // One command timeout for the whole tail, not one per reply
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
    assert!(
        target.link.is_none(),
        "the link is kept after a reply that never came"
    );
    assert!(target.pending.is_empty() && target.request.is_empty());
}

#[test]
fn flush_target_without_a_link_refuses_without_writing() {
    let c = config();
    let mut target = new_link(0, &c, &c.clients[0], &None);
    append_rpush(&mut target.request, b"dst", b"x");
    target.pending.push(7);
    let (replies, error) = flush_target(&mut target);
    assert_eq!(replies, vec![(7, Reply::Refused)]);
    assert_eq!(error.as_deref(), Some("not connected"));
    assert!(target.request.is_empty());
}

/// How a recording stand-in answers RPUSH (the sender's chunk tests)
#[derive(Clone, Copy, Debug)]
enum Answer {
    Int,
    Error,
    /// No reply, and the connection is read on (M3R-33)
    Never,
    /// No reply until this many RPUSHes have come, then all of them at once: a sender that
    /// waits for each reply before it writes the next gets none
    AfterAll(usize),
    /// RPUSH answered, LLEN refused with an error reply
    LlenError,
    /// The connection closed at the first RPUSH, which is recorded and never answered
    Close,
    /// RPUSH answered, LLEN answered far above any hardlimit: the target is stuck
    Full,
    /// An error reply to an RPUSH of a package numbered from the first to below the second
    /// (`pkgNNNN`), an integer to the others: a target that fails or is fixed at a point
    /// of the stream
    ErrorIn(usize, usize),
}

/// What a recording stand-in got, held by its test. The stand-in lives as long as a
/// handle does: once the test drops the last one its listener closes, and each of its
/// connections ends when its client closes. Before, no stand-in ever closed, so a test
/// process only gathered sockets and ran out of file descriptors at the common soft
/// limit of 1024 with 12 test threads
#[derive(Clone)]
struct Record(Arc<Recorded>);

struct Recorded {
    commands: Mutex<Vec<(String, Vec<u8>)>>,
    port: u16,
}

impl std::ops::Deref for Record {
    type Target = Mutex<Vec<(String, Vec<u8>)>>;
    fn deref(&self) -> &Self::Target {
        &self.0.commands
    }
}

impl Drop for Recorded {
    fn drop(&mut self) {
        // A port taken again by a later stand-in must not keep the short read timeout
        NEVER_ANSWERS.lock().unwrap().retain(|&p| p != self.port);
        // Wakes the accept loop, which finds no handle left and closes the listener
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// A stand-in target that records every command (name, last argument) in arrival order;
/// +OK to anything but RPUSH and LLEN (0)
fn recording_target(answer: Answer) -> (u16, Record) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    if matches!(answer, Answer::Never) {
        NEVER_ANSWERS.lock().unwrap().push(port);
    }
    let record = Record(Arc::new(Recorded {
        commands: Mutex::new(Vec::new()),
        port,
    }));
    let weak = Arc::downgrade(&record.0);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // The test dropped its last handle: no client of it is left to serve
            if weak.strong_count() == 0 {
                return;
            }
            let rec = weak.clone();
            // Each reply is its own write: without this a pipelined chunk waits on the
            // client's delayed ACK at every reply after the first
            let _ = stream.set_nodelay(true);
            thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                let mut r = BufReader::new(stream);
                let mut queued = 0;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(n) = line.trim_start_matches('*').trim().parse::<usize>() else {
                        return;
                    };
                    let mut args = Vec::new();
                    for _ in 0..n {
                        line.clear();
                        r.read_line(&mut line).unwrap();
                        let len: usize = line.trim_start_matches('$').trim().parse().unwrap();
                        let mut buf = vec![0; len + 2];
                        r.read_exact(&mut buf).unwrap();
                        buf.truncate(len);
                        args.push(buf);
                    }
                    let cmd = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
                    let Some(rec) = rec.upgrade() else {
                        return;
                    };
                    rec.commands
                        .lock()
                        .unwrap()
                        .push((cmd.clone(), args.last().unwrap().clone()));
                    drop(rec);
                    let reply = match (cmd.as_str(), answer) {
                        ("LLEN", Answer::LlenError) => "-ERR no length\r\n".to_string(),
                        ("LLEN", Answer::Full) => ":1000000\r\n".to_string(),
                        ("LLEN", _) => ":0\r\n".to_string(),
                        ("RPUSH", Answer::Full) => ":1\r\n".to_string(),
                        ("RPUSH", Answer::ErrorIn(from, to)) => {
                            let number = std::str::from_utf8(&args[2][3..])
                                .ok()
                                .and_then(|n| n.parse::<usize>().ok());
                            if number.is_some_and(|n| (from..to).contains(&n)) {
                                "-ERR refused\r\n".to_string()
                            } else {
                                ":1\r\n".to_string()
                            }
                        }
                        ("RPUSH", Answer::LlenError) => ":1\r\n".to_string(),
                        ("RPUSH", Answer::Int) => ":1\r\n".to_string(),
                        ("RPUSH", Answer::Error) => "-ERR refused\r\n".to_string(),
                        ("RPUSH", Answer::Never) => continue,
                        ("RPUSH", Answer::Close) => return,
                        ("RPUSH", Answer::AfterAll(k)) => {
                            queued += 1;
                            if queued < k {
                                continue;
                            }
                            ":1\r\n".repeat(k)
                        }
                        _ => "+OK\r\n".to_string(),
                    };
                    if w.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, record)
}

/// The packages a recording stand-in got by RPUSH
fn pushed_to(record: &Record) -> Vec<Vec<u8>> {
    record
        .lock()
        .unwrap()
        .iter()
        .filter(|(c, _)| c == "RPUSH")
        .map(|(_, v)| v.clone())
        .collect()
}

/// The ports of the stand-ins that never answer an RPUSH: connect_before_the_run() gives
/// their links a short read timeout once connected (M3R-57, M3F-26)
static NEVER_ANSWERS: Mutex<Vec<u16>> = Mutex::new(Vec::new());

/// The command timeout of a stand-in: far above any handshake or reply under load, so a
/// slow machine never turns a stand-in that answers into a failing one (M3F-26)
const STANDIN_TIMEOUT: f64 = 10.0;

/// The read timeout of a stand-in that never answers, set once it is connected: the only
/// wait it costs
const NEVER_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// A configuration whose targets are the given local ports
fn chunk_config(mode: &str, ports: &[u16]) -> Config {
    let mut c = config();
    c.mode = mode.to_string();
    c.clients = ports
        .iter()
        .enumerate()
        .map(|(i, &port)| {
            let mut t = client(&format!("t{i}"), &format!("dst{i}"));
            t.hostname = "127.0.0.1".into();
            t.port = port;
            t.command_timeout = Some(STANDIN_TIMEOUT);
            t
        })
        .collect();
    c
}

/// Connects a target before a run, waiting for the attempt however long it takes (up to
/// a minute): ensure_link alone waits CONNECT_GRACE (100 ms), and under load a first
/// connect that took longer failed the run "not connected" (M3F-25, M3R-56)
fn connect_before_the_run(l: &mut RedisLink) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !ensure_link(1, l) {
        // An attempt that ended without a link leaves its retry time set and none running
        assert!(
            l.connecting.is_some() || l.retry_at.is_none(),
            "not connected: the attempt failed ({} attempts)",
            l.attempts
        );
        assert!(Instant::now() < deadline, "not connected after a minute");
        thread::sleep(Duration::from_millis(10));
    }
    // By the port the link connected to, not its configuration's: a test that moves a link
    // to another stand-in keeps the configuration (M3R-60)
    if NEVER_ANSWERS.lock().unwrap().contains(&l.endpoint.port) {
        l.link
            .as_mut()
            .unwrap()
            .set_read_timeout(Some(NEVER_READ_TIMEOUT))
            .unwrap();
    }
}

/// One chunk through the sender's path, then every target without a link kept down for
/// the rest of the test whatever it lasts (its retry an hour ahead): a target that never
/// answers or closes does not come back after its 1 s backoff at a place that depends on
/// the machine's speed (M3R-56's rule for the other stand-in tests, M3F-26). A test that
/// wants one back says so by clearing its retry time.
fn run_chunk(
    c: &Config,
    links: &mut [RedisLink],
    cursor: &mut usize,
    chunk: &[Vec<u8>],
    counts: &mut Counts,
) {
    deliver_chunk(1, &None, c, links, cursor, chunk, counts);
    for l in links.iter_mut().filter(|l| l.link.is_none()) {
        l.retry_at = Some(Instant::now() + Duration::from_secs(3600));
    }
}

fn connected_links(c: &Config) -> Vec<RedisLink> {
    let mut links: Vec<RedisLink> = c.clients.iter().map(|t| new_link(1, c, t, &None)).collect();
    for l in links.iter_mut() {
        connect_before_the_run(l);
    }
    links
}

fn packages_n(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| format!("pkg{i:04}").into_bytes()).collect()
}

#[test]
fn a_chunk_is_written_before_its_replies_are_read() {
    // M3R-14: the replies come only once the whole chunk was written; a sender that waits
    // for each reply before the next RPUSH gets none and loses the chunk
    let (port, record) = recording_target(Answer::AfterAll(10));
    let c = chunk_config("replicant", &[port]);
    let mut links = connected_links(&c);
    let chunk = packages_n(10);
    let mut counts = Counts::default();
    run_chunk(&c, &mut links, &mut 0, &chunk, &mut counts);
    assert_eq!((counts.outgoing, counts.dropped), (10, 0));
    assert_eq!(
        pushed_to(&record),
        chunk,
        "not in order, or not every package"
    );
}

#[test]
fn spreader_plans_again_what_a_target_refused_or_never_answered() {
    // P2 = p2a and P1 = p1b: a package refused by a target (error reply) or whose reply
    // never came goes to the next target in another pass; never twice to one target
    for answer in [Answer::Error, Answer::Never] {
        let (bad, bad_rec) = recording_target(answer);
        let (good, good_rec) = recording_target(Answer::Int);
        let c = chunk_config("spreader", &[bad, good]);
        let mut links = connected_links(&c);
        let chunk = packages_n(10);
        let mut counts = Counts::default();
        let mut cursor = 0;
        run_chunk(&c, &mut links, &mut cursor, &chunk, &mut counts);
        assert_eq!((counts.outgoing, counts.dropped), (10, 0));
        let mut at_good = pushed_to(&good_rec);
        at_good.sort();
        assert_eq!(at_good, chunk, "the good target misses packages");
        let at_bad = pushed_to(&bad_rec);
        let mut unique = at_bad.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), at_bad.len(), "a package twice on one target");
        // The first pass alternated: half of the chunk was planned for the bad target
        assert_eq!(at_bad.len(), 5);
    }
}

#[test]
fn replicant_counts_an_unanswered_target_failed_and_never_resends_to_it() {
    // P1 = p1b, replicant: the other target takes every package (outgoing); the one whose
    // replies never came is not sent them again; its link is dropped
    let (mute, mute_rec) = recording_target(Answer::Never);
    let (good, _good_rec) = recording_target(Answer::Int);
    let c = chunk_config("replicant", &[mute, good]);
    let mut links = connected_links(&c);
    let chunk = packages_n(10);
    let mut counts = Counts::default();
    run_chunk(&c, &mut links, &mut 0, &chunk, &mut counts);
    assert_eq!((counts.outgoing, counts.dropped), (10, 0));
    assert_eq!(pushed_to(&mute_rec), chunk, "written once, in order");
    assert!(links[0].link.is_none() && links[1].link.is_some());
    // Every package lost on the only target: dropped as failed, once each
    let (mute, _mute_rec) = recording_target(Answer::Never);
    let c = chunk_config("replicant", &[mute]);
    let mut links = connected_links(&c);
    let mut counts = Counts::default();
    run_chunk(&c, &mut links, &mut 0, &chunk, &mut counts);
    assert_eq!(
        (counts.outgoing, counts.dropped, counts.failed),
        (0, 10, 10)
    );
}

#[test]
fn a_limits_check_reads_the_pending_requests_first() {
    // P4 = p4a: a check that is due writes and reads what is pending for that target
    // before its LLEN, so the length includes every package sent before. checklimit 3:
    // a check every 3 packages; without the flush every LLEN would come before any RPUSH
    let (port, record) = recording_target(Answer::Int);
    let mut c = chunk_config("replicant", &[port]);
    let t = &mut c.clients[0];
    (t.timelimit, t.checklimit, t.softlimit, t.hardlimit) =
        (Some(3600), Some(3), Some(50), Some(100));
    let mut links = connected_links(&c);
    let chunk = packages_n(10);
    let mut counts = Counts::default();
    run_chunk(&c, &mut links, &mut 0, &chunk, &mut counts);
    assert_eq!(counts.outgoing, 10);
    let seq: Vec<String> = record
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.clone())
        .filter(|c| c == "RPUSH" || c == "LLEN")
        .collect();
    let llens: Vec<usize> = seq
        .iter()
        .enumerate()
        .filter(|(_, c)| *c == "LLEN")
        .map(|(i, _)| i)
        .collect();
    assert!(llens.len() >= 3, "{seq:?}");
    for (k, &at) in llens.iter().enumerate().skip(1) {
        let before = seq[..at].iter().filter(|c| *c == "RPUSH").count();
        assert!(
            before >= 3 * k - 2,
            "LLEN {k} after only {before} RPUSH: {seq:?}"
        );
    }
}

#[test]
fn a_limits_check_that_fails_drops_the_package_as_failed() {
    // A due LLEN refused by the target: that package is not sent there (failed), the
    // others go on; a due LLEN whose flush finds the link gone: every package is failed
    // there, and nothing hangs
    for answer in [Answer::LlenError, Answer::Never] {
        let (port, record) = recording_target(answer);
        let mut c = chunk_config("replicant", &[port]);
        let t = &mut c.clients[0];
        (t.timelimit, t.checklimit, t.softlimit, t.hardlimit) =
            (Some(3600), Some(3), Some(50), Some(100));
        let mut links = connected_links(&c);
        let chunk = packages_n(10);
        let mut counts = Counts::default();
        run_chunk(&c, &mut links, &mut 0, &chunk, &mut counts);
        let llens = record
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, _)| c == "LLEN")
            .count() as u64;
        match answer {
            Answer::LlenError => {
                assert!(llens >= 3, "{llens}");
                assert_eq!((counts.dropped, counts.failed), (llens, llens));
                assert_eq!(counts.outgoing, 10 - llens);
            }
            _ => assert_eq!(
                (counts.outgoing, counts.dropped, counts.failed),
                (0, 10, 10)
            ),
        }
    }
}

/// Packages `from..from + n` as in packages_n
fn packages_from(from: usize, n: usize) -> Vec<Vec<u8>> {
    (from..from + n)
        .map(|i| format!("pkg{i:04}").into_bytes())
        .collect()
}

/// C4b-1 = 1c: spreader over three targets, the one at `bad` refusing every RPUSH
/// (`Answer::Error`) or closing at the first (`Answer::Close`); two chunks of 30 packages
/// from cursor 0, so the turn is t0, t1, t2. Returns what each good target got, by index
fn spread_two_chunks(bad: usize, answer: Answer) -> (Vec<Vec<Vec<u8>>>, Counts) {
    let stands: Vec<(u16, Record)> = (0..3)
        .map(|t| recording_target(if t == bad { answer } else { Answer::Int }))
        .collect();
    let ports: Vec<u16> = stands.iter().map(|(p, _)| *p).collect();
    let c = chunk_config("spreader", &ports);
    let mut links = connected_links(&c);
    let mut counts = Counts::default();
    let mut cursor = 0;
    for from in [0, 30] {
        let chunk = packages_from(from, 30);
        run_chunk(&c, &mut links, &mut cursor, &chunk, &mut counts);
    }
    // Never twice to one target (P1 = p1b, P2 = p2a)
    let at_bad = pushed_to(&stands[bad].1);
    let mut unique = at_bad.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        at_bad.len(),
        "a package twice on the failing target"
    );
    let got = stands
        .iter()
        .enumerate()
        .map(|(t, (_, rec))| if t == bad { Vec::new() } else { pushed_to(rec) })
        .collect();
    (got, counts)
}

/// Every package 0..60 exactly once over the good targets
fn every_package_once(got: &[Vec<Vec<u8>>]) {
    let mut all: Vec<Vec<u8>> = got.concat();
    all.sort();
    assert_eq!(all, packages_from(0, 60), "not every package exactly once");
}

/// Places where a target got a package after a later one
fn breaks(list: &[Vec<u8>]) -> usize {
    list.windows(2).filter(|w| w[0] > w[1]).count()
}

#[test]
fn spreader_keeps_the_order_when_a_target_before_the_last_fails() {
    // 1c: the packages the failing target refused or never answered join the next
    // targets' own before those are written, in release order: no break at any target
    for answer in [Answer::Error, Answer::Close] {
        for bad in [0, 1] {
            let (got, counts) = spread_two_chunks(bad, answer);
            every_package_once(&got);
            assert_eq!((counts.outgoing, counts.dropped), (60, 0));
            for (t, list) in got.iter().enumerate() {
                assert_eq!(
                    breaks(list),
                    0,
                    "target {t}, the failing one at {bad}: {list:?}"
                );
            }
        }
    }
}

#[test]
fn spreader_breaks_the_order_once_when_the_last_target_fails() {
    // The residual (plan 9.11.13 part 4, declared): the target written last in the turn
    // refuses in its replies, no target is left unwritten in that chunk, so its packages go
    // in another pass to the first target of the turn, after that target's own share: one
    // break there, at most the packages refused in that chunk late. From the next chunk the
    // failing target is written first (or it is down), so no break again
    for answer in [Answer::Error, Answer::Close] {
        let (got, counts) = spread_two_chunks(2, answer);
        every_package_once(&got);
        assert_eq!((counts.outgoing, counts.dropped), (60, 0));
        assert_eq!(breaks(&got[1]), 0, "{:?}", got[1]);
        let t0 = &got[0];
        assert_eq!(breaks(t0), 1, "{t0:?}");
        // The first chunk at t0: its own share (0, 3, ...) then t2's (2, 5, ...)
        let first: Vec<Vec<u8>> = t0
            .iter()
            .filter(|p| p.as_slice() < b"pkg0030".as_slice())
            .cloned()
            .collect();
        let own: Vec<Vec<u8>> = (0..30)
            .step_by(3)
            .map(|i| format!("pkg{i:04}").into_bytes())
            .collect();
        let late: Vec<Vec<u8>> = (2..30)
            .step_by(3)
            .map(|i| format!("pkg{i:04}").into_bytes())
            .collect();
        assert_eq!(first, [own, late.clone()].concat(), "{answer:?}");
        // At most the failing target's share of one chunk late
        assert_eq!(late.len(), 10);
        // The second chunk keeps the order at every target
        let second: Vec<Vec<u8>> = t0
            .iter()
            .filter(|p| p.as_slice() >= b"pkg0030".as_slice())
            .cloned()
            .collect();
        assert_eq!(breaks(&second), 0, "{answer:?}: {second:?}");
        assert!(
            t0.iter()
                .position(|p| p.as_slice() >= b"pkg0030".as_slice())
                .unwrap()
                == first.len()
        );
    }
}

/// Spreader over three recording targets, two chunks of 30 from cursor 0 (turn t0, t1,
/// t2); with `down` t2 is on a port nobody listens on, else `last` sets t2 up before the
/// first chunk
fn spread_with_last(down: bool, last: impl FnOnce(&mut RedisLink)) -> (Vec<Vec<Vec<u8>>>, Counts) {
    let stands: Vec<(u16, Record)> = (0..3).map(|_| recording_target(Answer::Int)).collect();
    let mut ports: Vec<u16> = stands.iter().map(|(p, _)| *p).collect();
    if down {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        ports[2] = closed.local_addr().unwrap().port();
    }
    let c = chunk_config("spreader", &ports);
    let mut links: Vec<RedisLink> = c
        .clients
        .iter()
        .map(|t| new_link(1, &c, t, &None))
        .collect();
    for l in links.iter_mut().take(if down { 2 } else { 3 }) {
        connect_before_the_run(l);
    }
    last(&mut links[2]);
    let mut counts = Counts::default();
    let mut cursor = 0;
    for from in [0, 30] {
        let chunk = packages_from(from, 30);
        run_chunk(&c, &mut links, &mut cursor, &chunk, &mut counts);
    }
    (
        stands.iter().map(|(_, rec)| pushed_to(rec)).collect(),
        counts,
    )
}

#[test]
fn spreader_keeps_the_order_with_a_target_down() {
    // Refused at planning time (plan 9.11.13): the turn passes over it, nothing of its is
    // carried, no break even in the first chunk with it last in the turn
    let (got, counts) = spread_with_last(true, |_| {});
    assert!(got[2].is_empty());
    every_package_once(&got);
    assert_eq!((counts.outgoing, counts.dropped), (60, 0));
    for list in &got {
        assert_eq!(breaks(list), 0, "{list:?}");
    }
}

#[test]
fn spreader_keeps_the_order_with_a_filtered_target() {
    // A client filter that takes only even packages: the turn passes over it for the odd
    // ones (plan 9.11.13), no break
    let (got, counts) = spread_with_last(false, |l| {
        l.filter = Some(Filter::new("[02468]$", None, false, None, None).unwrap());
    });
    assert!(got[2].iter().all(|p| p.last().unwrap() % 2 == 0));
    assert!(!got[2].is_empty());
    let mut all: Vec<Vec<u8>> = got.concat();
    all.sort();
    assert_eq!(all, packages_from(0, 60), "not every package exactly once");
    assert_eq!((counts.outgoing, counts.dropped), (60, 0));
    for list in &got {
        assert_eq!(breaks(list), 0, "{list:?}");
    }
}

#[test]
fn spreader_never_sends_a_package_twice_to_one_target() {
    // Every target refuses: each package is tried once on each target and dropped as
    // failed, in the other pass too (P2 = p2a: never the same target twice)
    let stands: Vec<(u16, Record)> = (0..3).map(|_| recording_target(Answer::Error)).collect();
    let ports: Vec<u16> = stands.iter().map(|(p, _)| *p).collect();
    let c = chunk_config("spreader", &ports);
    let mut links = connected_links(&c);
    let mut counts = Counts::default();
    let mut cursor = 1;
    let chunk = packages_n(30);
    run_chunk(&c, &mut links, &mut cursor, &chunk, &mut counts);
    assert_eq!(
        (counts.outgoing, counts.dropped, counts.failed),
        (0, 30, 30)
    );
    for (_, rec) in &stands {
        let mut got = pushed_to(rec);
        got.sort();
        assert_eq!(got, chunk, "not every package once on every target");
    }
}

/// How the failing target of a shares case behaves (plan 9.11.13)
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fails {
    /// Refuses at planning time: stuck by its limits
    Stuck,
    /// Refuses at planning time: nobody listens on its port
    Down,
    /// Refuses in its replies: an error to every RPUSH
    Errors,
    /// Refuses in its replies: closes at its first RPUSH (then down)
    Closes,
    /// Refuses in its replies: never answers (a short command timeout, then down)
    Mute,
    /// Refuses in its replies: an error to the packages before the 80th, then takes them
    ErrorsFirst,
}

/// The stand-ins' answers in a shares pool
const SHARE_ANSWERS: [Answer; 6] = [
    Answer::Int,
    Answer::Full,
    Answer::Error,
    Answer::Close,
    Answer::Never,
    Answer::ErrorIn(0, 80),
];

/// Stand-ins for `targets` targets, one per answer of SHARE_ANSWERS each
fn shares_pool(targets: usize) -> Vec<Vec<(u16, Record)>> {
    (0..targets)
        .map(|_| SHARE_ANSWERS.iter().map(|&a| recording_target(a)).collect())
        .collect()
}

/// The client filters of the shares cases, by the package's last digit
const SHARE_FILTERS: [Option<&str>; 4] =
    [None, Some("[02468]$"), Some("[13579]$"), Some("[0369]$")];

const SHARE_PACKAGES: usize = 600;
const SHARE_CHUNK: usize = 50;

/// The command timeout of a shares stand-in that answers: far above any handshake or
/// reply under load, so a slow machine never turns a healthy target into a failing one
/// (M3F-25, M3R-56)
const SHARE_TIMEOUT: f64 = 10.0;

/// The reply timeout of a shares stand-in that never answers (Mute), set once it is
/// connected, and the command timeout of one nobody serves (Down): their only cost
const SHARE_MUTE_TIMEOUT: Duration = Duration::from_millis(200);

/// What each target got, in arrival order, and the counts: spreader over `filters.len()`
/// stand-ins, the one at `bad` failing as said, `SHARE_PACKAGES` packages from cursor 0;
/// `chunked`: through deliver_chunk() in chunks of `SHARE_CHUNK` (the sender), else one by
/// one through deliver() (the workers' rotation: 1.0.8's and the stage 4 tip's rule)
fn shares_run(
    pool: &[Vec<(u16, Record)>],
    filters: &[Option<&str>],
    bad: Option<(usize, Fails)>,
    chunked: bool,
) -> (Vec<Vec<Vec<u8>>>, Counts) {
    shares_run_with(pool, filters, bad.as_slice(), chunked)
}

/// As shares_run, with any number of failing targets (M3R-47: a held-back target beside a
/// stuck and a never-answering one)
fn shares_run_with(
    pool: &[Vec<(u16, Record)>],
    filters: &[Option<&str>],
    bads: &[(usize, Fails)],
    chunked: bool,
) -> (Vec<Vec<Vec<u8>>>, Counts) {
    let kind = |t: usize| bads.iter().find(|(b, _)| *b == t).map(|&(_, k)| k);
    // The stand-ins are reused from case to case (a fresh set each time runs out of file
    // descriptors): pool[t] holds one per answer, in the order of SHARE_ANSWERS
    let answer = |t: usize| match kind(t) {
        Some(Fails::Stuck) => 1,
        Some(Fails::Errors) => 2,
        Some(Fails::Closes) => 3,
        Some(Fails::Mute) => 4,
        Some(Fails::ErrorsFirst) => 5,
        _ => 0,
    };
    let stands: Vec<(u16, Record)> = (0..filters.len())
        .map(|t| pool[t][answer(t)].clone())
        .collect();
    for (_, rec) in &stands {
        rec.lock().unwrap().clear();
    }
    let mut ports: Vec<u16> = stands.iter().map(|(p, _)| *p).collect();
    let mut closed = Vec::new();
    for (t, port) in ports.iter_mut().enumerate() {
        if kind(t) == Some(Fails::Down) {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            *port = l.local_addr().unwrap().port();
            closed.push(l);
        }
    }
    let mut c = chunk_config("spreader", &ports);
    for (b, t) in c.clients.iter_mut().enumerate() {
        // Down: its connects are never served, so they cost this timeout each
        t.command_timeout = Some(if kind(b) == Some(Fails::Down) {
            SHARE_MUTE_TIMEOUT.as_secs_f64()
        } else {
            SHARE_TIMEOUT
        });
        if kind(b) == Some(Fails::Stuck) {
            (t.timelimit, t.checklimit, t.softlimit, t.hardlimit) =
                (Some(3600), Some(7), Some(50), Some(100));
        }
    }
    let mut links: Vec<RedisLink> = c
        .clients
        .iter()
        .zip(filters)
        .map(|(t, f)| {
            let filter = f.map(|re| Filter::new(re, None, false, None, None).unwrap());
            new_link(1, &c, t, &filter)
        })
        .collect();
    for (t, l) in links.iter_mut().enumerate() {
        if kind(t) != Some(Fails::Down) {
            connect_before_the_run(l);
        }
        if kind(t) == Some(Fails::Mute) {
            // Connected with the long timeout; only its replies, which never come, wait short
            l.link
                .as_mut()
                .unwrap()
                .set_read_timeout(Some(SHARE_MUTE_TIMEOUT))
                .unwrap();
        }
    }
    let mut counts = Counts::default();
    let mut cursor = 0;
    let packages = packages_n(SHARE_PACKAGES);
    // A target that never answers or that closes is down for the rest of the run, in both
    // runs alike, however long they take: its retry time is kept out of reach once it went
    // down (M3R-56: after its 1 s backoff it came back at a different package in each run)
    let keep_down = |links: &mut [RedisLink]| {
        for (t, l) in links.iter_mut().enumerate() {
            if matches!(kind(t), Some(Fails::Mute | Fails::Closes)) && l.link.is_none() {
                l.retry_at = Some(Instant::now() + Duration::from_secs(3600));
            }
        }
    };
    if chunked {
        for chunk in packages.chunks(SHARE_CHUNK) {
            deliver_chunk(1, &None, &c, &mut links, &mut cursor, chunk, &mut counts);
            keep_down(&mut links);
        }
    } else {
        for package in &packages {
            deliver(1, &None, &c, &mut links, &mut cursor, package, &mut counts);
            keep_down(&mut links);
        }
    }
    drop(closed);
    (
        stands.iter().map(|(_, rec)| pushed_to(rec)).collect(),
        counts,
    )
}

/// The number of a package (`pkgNNNN`)
fn number(p: &[u8]) -> usize {
    std::str::from_utf8(&p[3..]).unwrap().parse().unwrap()
}

/// Packages that reached a target after a later one
fn late(list: &[Vec<u8>]) -> usize {
    let mut top: Option<&Vec<u8>> = None;
    let mut n = 0;
    for p in list {
        if top.is_some_and(|t| p < t) {
            n += 1;
        } else {
            top = Some(p);
        }
    }
    n
}

/// One generated case: a client filter per target, and the failing target with its kind
type ShareCase = (Vec<Option<&'static str>>, Option<(usize, Fails)>);

/// The generated cases of one target count: every combination of SHARE_FILTERS (a seeded
/// sample of 48 for 4 targets), each with no failure and with each kind of failure at
/// each position (Mute on every fourth combination only: it costs its timeout)
fn share_cases(targets: usize) -> Vec<ShareCase> {
    let mut combos: Vec<Vec<Option<&'static str>>> = vec![vec![]];
    for _ in 0..targets {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                SHARE_FILTERS.iter().map(move |f| {
                    let mut c = c.clone();
                    c.push(*f);
                    c
                })
            })
            .collect();
    }
    if targets == 4 {
        // A fixed linear congruential sample: the same cases on every run
        let mut x: u64 = 42;
        let mut picked = Vec::new();
        while picked.len() < 48 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let i = (x >> 33) as usize % combos.len();
            if !picked.contains(&i) {
                picked.push(i);
            }
        }
        combos = picked.into_iter().map(|i| combos[i].clone()).collect();
    }
    let mut cases = Vec::new();
    for (k, f) in combos.into_iter().enumerate() {
        cases.push((f.clone(), None));
        for b in 0..targets {
            let mut kinds = vec![Fails::Stuck, Fails::Down, Fails::Errors, Fails::Closes];
            if k % 4 == 0 {
                kinds.push(Fails::Mute);
            }
            for kind in kinds {
                cases.push((f.clone(), Some((b, kind))));
            }
        }
    }
    cases
}

/// The differential of plan 9.11.13 (rule of 2026-11-16): spreader with ordering (the
/// sender's chunks) against the workers' rotation, over the generated cases of one target
/// count. A failure at planning time (none, stuck, down) gives each healthy target the
/// very packages the old rule gives it, in order; a failure in the replies (errors, a
/// close, no reply) changes the shares of one chunk at most, by at most that chunk (the
/// failing target's share of it, declared), and as many packages late at most. Every
/// package the old rule delivers is delivered, never twice to one target.
fn spreader_shares_match_the_workers_rotation(targets: usize) {
    use std::collections::BTreeMap;
    // class -> (cases, equal, largest difference, most late)
    let mut table: BTreeMap<String, (usize, usize, usize, usize, f64)> = BTreeMap::new();
    let pool = shares_pool(targets);
    for (filters, bad) in share_cases(targets) {
        let began = Instant::now();
        let (old, old_counts) = shares_run(&pool, &filters, bad, false);
        let (new, new_counts) = shares_run(&pool, &filters, bad, true);
        let what = format!("{filters:?} {bad:?}");
        let healthy: Vec<usize> = (0..targets)
            .filter(|&t| bad.is_none_or(|(b, _)| b != t))
            .collect();
        for list in &new {
            let mut u = list.clone();
            u.sort();
            u.dedup();
            assert_eq!(u.len(), list.len(), "a package twice on one target: {what}");
        }
        let union = |got: &[Vec<Vec<u8>>]| {
            let mut all: Vec<Vec<u8>> = healthy.iter().flat_map(|&t| got[t].clone()).collect();
            all.sort();
            all
        };
        let delivered = union(&new);
        let mut once = delivered.clone();
        once.dedup();
        assert_eq!(
            once.len(),
            delivered.len(),
            "on two healthy targets: {what}"
        );
        assert_eq!(
            delivered,
            union(&old),
            "not the same packages delivered: {what}"
        );
        assert_eq!(
            (new_counts.outgoing, new_counts.dropped),
            (old_counts.outgoing, old_counts.dropped),
            "{what}"
        );
        let mut equal = true;
        let mut diff = 0;
        let mut most_late = 0;
        for &t in &healthy {
            let mut a = old[t].clone();
            let mut b = new[t].clone();
            most_late = most_late.max(late(&b));
            a.sort();
            b.sort();
            equal &= a == b;
            diff = diff.max(a.len().abs_diff(b.len()));
        }
        let class = match bad {
            None => "none".to_string(),
            Some((_, kind)) => format!("{kind:?}"),
        };
        match bad {
            None | Some((_, Fails::Stuck | Fails::Down)) => {
                assert!(equal, "shares differ: {what}\nold {old:?}\nnew {new:?}");
                assert_eq!(most_late, 0, "out of order: {what}\n{new:?}");
            }
            _ => {
                assert!(diff <= SHARE_CHUNK, "{diff} apart: {what}");
                assert!(most_late <= SHARE_CHUNK, "{most_late} late: {what}");
            }
        }
        let row = table.entry(class).or_default();
        row.0 += 1;
        row.1 += usize::from(equal);
        row.2 = row.2.max(diff);
        row.3 = row.3.max(most_late);
        row.4 += began.elapsed().as_secs_f64();
    }
    for (class, (cases, equal, diff, most_late, secs)) in table {
        eprintln!(
            "\nshares {targets} targets | {class} | {cases} cases | {equal} equal | largest difference {diff} | most late {most_late} | {secs:.1} s"
        );
    }
}

#[test]
fn spreader_shares_match_the_workers_rotation_2_targets() {
    spreader_shares_match_the_workers_rotation(2);
}

#[test]
fn spreader_shares_match_the_workers_rotation_3_targets() {
    spreader_shares_match_the_workers_rotation(3);
}

#[test]
fn spreader_shares_match_the_workers_rotation_4_targets() {
    spreader_shares_match_the_workers_rotation(4);
}

/// The packages of `got` from `from` on, sorted
fn from_package(got: &[Vec<u8>], from: usize) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = got
        .iter()
        .filter(|p| p.as_slice() >= format!("pkg{from:04}").as_bytes())
        .cloned()
        .collect();
    v.sort();
    v
}

#[test]
fn spreader_with_a_filter_on_every_target_keeps_the_order() {
    // M3R-43: filters "even", "odd" and none (the review's three-filter case), and one on
    // every target: the shares are the workers' rotation's, no package late anywhere
    let pool = shares_pool(3);
    for filters in [
        [Some("[02468]$"), Some("[13579]$"), None],
        [Some("[02468]$"), Some("[13579]$"), Some("[0369]$")],
    ] {
        let (old, _) = shares_run(&pool, &filters, None, false);
        let (new, counts) = shares_run(&pool, &filters, None, true);
        assert_eq!(counts.outgoing, SHARE_PACKAGES as u64);
        for t in 0..3 {
            assert_eq!(late(&new[t]), 0, "{filters:?} t{t}: {:?}", new[t]);
            assert_eq!(
                from_package(&new[t], 0),
                from_package(&old[t], 0),
                "{filters:?} t{t}"
            );
        }
    }
}

#[test]
fn spreader_shares_as_the_old_rule_after_a_failure_event() {
    // Plan 9.11.13 part 3: the shares differ only in the chunk of the failure event. After
    // it the cursor is where the old rule leaves it (resync), and a target that keeps
    // answering errors is held back with one probe per chunk (C4b-2 = a). Filters that
    // depend on the position make any shift of the cursor show
    let pool = shares_pool(4);
    for filters in [
        vec![None, None, None, Some("[13579]$")],
        vec![Some("[02468]$"), None, Some("[13579]$")],
        vec![None, None, None],
    ] {
        for kind in [Fails::Errors, Fails::Closes, Fails::Mute] {
            let bad = Some((1, kind));
            let (old, _) = shares_run(&pool, &filters, bad, false);
            let (new, _) = shares_run(&pool, &filters, bad, true);
            for t in (0..filters.len()).filter(|&t| t != 1) {
                assert_eq!(
                    from_package(&new[t], SHARE_CHUNK),
                    from_package(&old[t], SHARE_CHUNK),
                    "{filters:?} {kind:?} t{t}"
                );
            }
            if kind == Fails::Errors {
                // Its share of the first chunk, then one probe per chunk
                let first = new[1].len() - from_package(&new[1], SHARE_CHUNK).len();
                assert_eq!(
                    new[1].len(),
                    first + SHARE_PACKAGES / SHARE_CHUNK - 1,
                    "{filters:?}: {:?}",
                    new[1]
                );
            }
        }
    }
}

#[test]
fn spreader_takes_back_a_held_target_after_its_probe() {
    // C4b-2 = a: a target fixed at package 80 (errors before) is held back after the first
    // chunk; the probe of the second chunk (the last package the turn offered it) is taken,
    // and from the third chunk it gets its share again, in order. The chunk in which it
    // comes back is one of those that differ (declared): only the totals are compared
    let pool = shares_pool(3);
    let bad = Some((1, Fails::ErrorsFirst));
    let filters = [None, None, None];
    let (old, old_counts) = shares_run(&pool, &filters, bad, false);
    let (new, new_counts) = shares_run(&pool, &filters, bad, true);
    assert_eq!(new_counts.outgoing, old_counts.outgoing);
    let second: Vec<&Vec<u8>> = new[1]
        .iter()
        .filter(|p| (b"pkg0050".as_slice()..b"pkg0100".as_slice()).contains(&p.as_slice()))
        .collect();
    assert_eq!(
        second.len(),
        1,
        "one probe in the second chunk: {:?}",
        new[1]
    );
    let back = from_package(&new[1], 2 * SHARE_CHUNK);
    assert!(
        back.len() >= (SHARE_PACKAGES - 2 * SHARE_CHUNK) / 3 - 1,
        "not its share again: {:?}",
        new[1]
    );
    assert!(!from_package(&old[1], 80).is_empty());
    for list in &new {
        assert_eq!(late(list), 0, "{list:?}");
    }
}

#[test]
fn pending_requests_stay_in_release_order() {
    // A request put before later ones lands before them, bytes and package alike
    let c = chunk_config("spreader", &[1]);
    let mut link = new_link(1, &c, &c.clients[0], &None);
    for p in [5, 1, 3, 9, 0] {
        push_request(&mut link, p, format!("v{p}").as_bytes());
    }
    assert_eq!(link.pending, vec![0, 1, 3, 5, 9]);
    let mut want = Vec::new();
    for p in [0, 1, 3, 5, 9] {
        append_rpush(&mut want, b"dst0", format!("v{p}").as_bytes());
    }
    assert_eq!(link.request, want);
    assert_eq!(link.ends.last(), Some(&want.len()));
    let (replies, _) = flush_target(&mut link);
    assert!(link.request.is_empty() && link.ends.is_empty() && link.pending.is_empty());
    assert_eq!(replies.len(), 5);
}

/// Spreader over stand-ins answering as given, `chunks` chunks of `size` from cursor 0,
/// with the client filters given; what each got
fn spread_answers(
    answers: &[Answer],
    filters: &[Option<&str>],
    chunks: usize,
    size: usize,
) -> Vec<Vec<Vec<u8>>> {
    let stands: Vec<(u16, Record)> = answers.iter().map(|&a| recording_target(a)).collect();
    let ports: Vec<u16> = stands.iter().map(|(p, _)| *p).collect();
    let c = chunk_config("spreader", &ports);
    let mut links: Vec<RedisLink> = c
        .clients
        .iter()
        .zip(filters)
        .map(|(t, f)| {
            let filter = f.map(|re| Filter::new(re, None, false, None, None).unwrap());
            new_link(1, &c, t, &filter)
        })
        .collect();
    for l in links.iter_mut() {
        connect_before_the_run(l);
    }
    let mut counts = Counts::default();
    let mut cursor = 0;
    for chunk in packages_n(chunks * size).chunks(size) {
        run_chunk(&c, &mut links, &mut cursor, chunk, &mut counts);
    }
    stands.iter().map(|(_, rec)| pushed_to(rec)).collect()
}

#[test]
fn spreader_carries_only_to_targets_that_wrote_nothing_later() {
    // Plan 9.11.13 part 2. A: t2 answers errors to everything (held back, written first
    // from the second chunk); chunks of 48 leave the cursor at t0, so the turn is t2, t0,
    // t1, t3. t1 answers errors in the second chunk: what it refuses goes to t3, not yet
    // written, rather than to t2, which wrote its probe (a later package) and would refuse
    // it anyway, after which t3 would have written later ones
    let got = spread_answers(
        &[
            Answer::Int,
            Answer::ErrorIn(48, 96),
            Answer::Error,
            Answer::Int,
        ],
        &[None, None, None, None],
        3,
        48,
    );
    for t in [0, 3] {
        assert_eq!(late(&got[t]), 0, "t{t}: {:?}", got[t]);
    }
    // B: only a refusal in the replies puts a target first in the turn. t2 has a filter
    // (packages whose last digit is 0, 3, 6 or 9); t1 answers errors in
    // the second chunk: written in the cursor's turn, t2 is still unwritten and takes what
    // t1 refused that it accepts, in its place (t0 gets the rest late: the residual)
    let got = spread_answers(
        &[Answer::Int, Answer::ErrorIn(30, 60), Answer::Int],
        &[None, None, Some("[0369]$")],
        3,
        30,
    );
    assert_eq!(late(&got[2]), 0, "t2: {:?}", got[2]);
}

#[test]
fn repeated_warning_prints_once_per_window_and_counts_the_rest() {
    let t0 = Instant::now();
    let mut r = Repeated::default();
    let w = TS_MISSING;
    // The first at once, then nothing for REPEAT_WINDOW
    assert_eq!(r.hit(w, t0).as_deref(), Some(w));
    for k in 1..1000u32 {
        assert_eq!(r.hit(w, t0 + Duration::from_millis(u64::from(k))), None);
    }
    // After the window: one line for the 999 not printed and this one
    let line = r.hit(w, t0 + REPEAT_WINDOW).unwrap();
    assert_eq!(line, format!("{} (1000 since the last line)", w));
    // A line due with nothing kept back is the warning alone
    assert_eq!(r.hit(w, t0 + REPEAT_WINDOW * 3).as_deref(), Some(w));
    assert_eq!(r.left(w), None);
    // Those kept back when the thread ends: one line with their count
    r.hit(w, t0 + REPEAT_WINDOW * 3 + Duration::from_millis(1));
    r.hit(w, t0 + REPEAT_WINDOW * 3 + Duration::from_millis(2));
    assert_eq!(r.left(w), Some(format!("{} (2 since the last line)", w)));
    assert_eq!(r.left(w), None);
}

#[test]
fn repeated_warnings_of_two_kinds_do_not_share_a_count() {
    let t0 = Instant::now();
    let mut kinds = [Repeated::default(), Repeated::default()];
    assert!(kinds[0].hit(TS_MISSING, t0).is_some());
    assert!(kinds[1].hit(TS_UNPARSED, t0).is_some());
    assert!(kinds[0].hit(TS_MISSING, t0).is_none());
    assert_eq!(kinds[1].left(TS_UNPARSED), None);
    assert_eq!(
        kinds[0].left(TS_MISSING),
        Some(format!("{} (1 since the last line)", TS_MISSING))
    );
}

#[test]
fn a_line_is_built_whole_with_colours_only_when_asked() {
    let mut on = Vec::new();
    one_colour(
        &mut on,
        true,
        COLOR_RED,
        0,
        format_args!("t0 :: {} stuck!", "dst"),
    );
    assert!(
        on.starts_with(COLOR_RED),
        "{:?}",
        String::from_utf8_lossy(&on)
    );
    assert!(
        on.ends_with(b"t0 :: dst stuck!\x1B[1;0m"),
        "{:?}",
        String::from_utf8_lossy(&on)
    );
    assert_eq!(
        on.windows(18)
            .filter(|w| w == b"RedisMultiplexer: ")
            .count(),
        1
    );
    let mut off = Vec::new();
    one_colour(
        &mut off,
        false,
        COLOR_RED,
        0,
        format_args!("t0 :: {} stuck!", "dst"),
    );
    assert!(!off.contains(&0x1B), "{:?}", String::from_utf8_lossy(&off));
    assert!(off.ends_with(b"RedisMultiplexer: t0 :: dst stuck!"));
    // The same text without the codes
    let stripped: Vec<u8> = String::from_utf8_lossy(&on)
        .replace("\x1B[1;31m", "")
        .replace("\x1B[1;0m", "")
        .into_bytes();
    assert_eq!(stripped, off);
    // No head
    let mut bare = Vec::new();
    one_colour(
        &mut bare,
        false,
        COLOR_CYAN,
        COLOR_NOHEAD,
        format_args!("Usage"),
    );
    assert_eq!(bare, b"Usage");
    // A part of a line: its colour, its text, the closing code
    let mut p = Vec::new();
    part(&mut p, true, COLOR_WHITE, format_args!(" | "));
    assert_eq!(
        p,
        [COLOR_WHITE.as_slice(), b" | ", COLOR_CLOSE.as_slice()].concat()
    );
    part(&mut p, false, COLOR_WHITE, format_args!(" | "));
    assert!(p.ends_with(b"\x1B[1;0m | "));
}

/// M3R-47: a held-back target (errors to everything) beside two other failing ones and
/// two healthy ones, 5 targets, the held-back target at each place with each client filter
/// and one healthy target filtered in turn. Two healthy targets keep the rotation
/// observable (with one, it takes every package whatever the cursor says). After the chunks
/// of the failure events (the first two) each target that takes packages gets exactly what
/// the workers' rotation gives it: the hold-back and the resync take a stuck target, a lost
/// link and a filter as 1.0.8 does. A refusing target is not compared (a held-back one gets
/// one probe per chunk where 1.0.8 offered it every package, declared, and that is
/// asserted), but no target gets a package its filter refuses, and the counts match. A
/// target that errors to the first packages only is released at the end of the second
/// chunk or the third and compared like the others from the fourth.
fn hold_back_beside(roles: [Fails; 3]) -> usize {
    const TARGETS: usize = 5;
    let pool = shares_pool(TARGETS);
    // A target fixed at package 80 (the second chunk) comes back to its share up to one
    // chunk later (declared): compared from the fourth chunk
    let settled = if roles.contains(&Fails::ErrorsFirst) {
        3
    } else {
        2
    } * SHARE_CHUNK;
    let mut cases = 0;
    for at in 0..TARGETS {
        let others: Vec<usize> = (1..TARGETS).map(|k| (at + k) % TARGETS).collect();
        for (k, f) in SHARE_FILTERS.iter().enumerate() {
            let mut filters = [None; TARGETS];
            filters[at] = *f;
            // One of the healthy targets (others[2], others[3]) filtered, by turns
            filters[others[2 + k % 2]] = SHARE_FILTERS[(k + 1) % 4];
            let bads = [(at, roles[0]), (others[0], roles[1]), (others[1], roles[2])];
            let (old, old_counts) = shares_run_with(&pool, &filters, &bads, false);
            let (new, new_counts) = shares_run_with(&pool, &filters, &bads, true);
            let what = format!("{filters:?} {bads:?}");
            for (t, got) in new.iter().enumerate() {
                if let Some(re) = filters[t] {
                    let re = regex::bytes::Regex::new(re).unwrap();
                    assert!(
                        got.iter().all(|p| re.is_match(p)),
                        "target {t} got a package its filter refuses: {what}"
                    );
                }
            }
            // A target held back for good gets one probe per chunk from the second on, also
            // when others are held back beside it (M3F-29: the probe refused by one is
            // offered to the next, as 1.0.8 offers it)
            for &(b, _) in bads.iter().filter(|&&(_, k)| k == Fails::Errors) {
                for ch in 1..SHARE_PACKAGES / SHARE_CHUNK {
                    let probes = new[b]
                        .iter()
                        .filter(|p| number(p) / SHARE_CHUNK == ch)
                        .count();
                    assert_eq!(probes, 1, "target {b}, probes in chunk {ch}: {what}");
                }
            }
            let refusing = |t: usize| {
                bads.iter().any(|&(b, k)| {
                    b == t && matches!(k, Fails::Errors | Fails::Mute | Fails::Closes)
                })
            };
            for t in (0..TARGETS).filter(|&t| !refusing(t)) {
                assert_eq!(
                    from_package(&new[t], settled),
                    from_package(&old[t], settled),
                    "target {t} after the event chunks: {what}"
                );
            }
            assert_eq!(new_counts, old_counts, "counts: {what}");
            cases += 1;
        }
    }
    cases
}

#[test]
fn spreader_hold_back_beside_a_stuck_and_a_silent_target() {
    let n = hold_back_beside([Fails::Errors, Fails::Stuck, Fails::Mute]);
    eprintln!("\nhold-back beside stuck and mute: {n} cases");
}

#[test]
fn spreader_release_beside_a_down_and_a_closing_target() {
    let n = hold_back_beside([Fails::ErrorsFirst, Fails::Down, Fails::Closes]);
    eprintln!("\nrelease beside down and closing: {n} cases");
}

#[test]
fn spreader_release_beside_a_stuck_and_a_silent_target() {
    let n = hold_back_beside([Fails::ErrorsFirst, Fails::Stuck, Fails::Mute]);
    eprintln!("\nrelease beside stuck and mute: {n} cases");
}

#[test]
fn spreader_hold_back_beside_a_down_and_a_stuck_target() {
    let n = hold_back_beside([Fails::Errors, Fails::Down, Fails::Stuck]);
    eprintln!("\nhold-back beside down and stuck: {n} cases");
}

#[test]
fn spreader_hold_back_two_refusing_targets_side_by_side() {
    // M3F-29: each of two held-back targets next to each other meets a probe in every
    // chunk (one package, offered to the second once the first refused it)
    let n = hold_back_beside([Fails::Errors, Fails::Errors, Fails::Down]);
    eprintln!("\nhold-back of two side by side: {n} cases");
}

#[test]
fn spreader_release_beside_a_held_back_target() {
    // M3F-29: a target fixed while the one before it keeps refusing comes back to its
    // share, as the old rule gives it, at most one chunk late
    let n = hold_back_beside([Fails::Errors, Fails::ErrorsFirst, Fails::Stuck]);
    eprintln!("\nrelease beside a held-back target: {n} cases");
}

#[test]
fn spreader_hold_back_three_refusing_targets_in_a_row() {
    // M3F-29, R1: a chain of three held-back targets, each its probe per chunk
    let n = hold_back_beside([Fails::Errors, Fails::Errors, Fails::Errors]);
    eprintln!("\nhold-back of three in a row: {n} cases");
}

#[test]
fn spreader_hold_back_two_refusing_targets_around_a_stuck_one() {
    // M3F-29, R1: a stuck target between them takes nothing: the chain passes over it
    let n = hold_back_beside([Fails::Errors, Fails::Stuck, Fails::Errors]);
    eprintln!("\nhold-back around a stuck target: {n} cases");
}

#[test]
fn spreader_hold_back_two_refusing_targets_around_a_down_one() {
    // M3F-29, R1: the same with a target down between them
    let n = hold_back_beside([Fails::Errors, Fails::Down, Fails::Errors]);
    eprintln!("\nhold-back around a down target: {n} cases");
}

#[test]
fn spreader_hold_back_two_refusing_targets_around_a_filtered_one() {
    // M3F-29, R1: the same with a target between them whose filter takes nothing; each
    // refusing target meets one package per chunk from the second on
    let got = spread_answers(
        &[Answer::Error, Answer::Int, Answer::Error, Answer::Int],
        &[None, Some("^none$"), None, None],
        6,
        30,
    );
    assert!(got[1].is_empty(), "the filtered target took packages");
    for t in [0, 2] {
        for ch in 1..6 {
            let probes = got[t].iter().filter(|p| number(p) / 30 == ch).count();
            assert_eq!(probes, 1, "target {t}, probes in chunk {ch}");
        }
    }
}

#[test]
fn spreader_with_every_target_held_back_offers_each_package_once_to_each() {
    // M3F-29, R1: nobody to give a probe back to (M3R-58's arm): every package is tried
    // by the refusing targets, never twice by one, and none is lost unaccounted
    let got = spread_answers(
        &[Answer::Error, Answer::Error, Answer::Error],
        &[None; 3],
        4,
        30,
    );
    let mut seen = vec![0; 120];
    for list in &got {
        let mut mine: Vec<usize> = list.iter().map(|p| number(p)).collect();
        let len = mine.len();
        mine.sort_unstable();
        mine.dedup();
        assert_eq!(mine.len(), len, "a package twice to one target");
        for n in mine {
            seen[n] += 1;
        }
    }
    assert!(
        seen.iter().all(|&k| k >= 1),
        "a package offered to no target"
    );
}

/// The chunked run against the old rule for one case (the review's generated cases):
/// the same counts, the same packages delivered by the targets that do not refuse
/// everything (the shares may differ in a chunk with a refusal), nothing twice on any
/// target
fn chunked_as_the_old_rule(filters: &[Option<&'static str>], bads: &[(usize, Fails)]) {
    let pool = shares_pool(filters.len());
    let old = shares_run_with(&pool, filters, bads, false);
    let new = shares_run_with(&pool, filters, bads, true);
    assert_eq!(
        (new.1.outgoing, new.1.dropped),
        (old.1.outgoing, old.1.dropped),
        "counts (outgoing, dropped)"
    );
    let mut seen_old = std::collections::BTreeSet::new();
    let mut seen_new = std::collections::BTreeSet::new();
    for t in 0..filters.len() {
        let mut mine: Vec<usize> = new.0[t].iter().map(|p| number(p)).collect();
        let len = mine.len();
        mine.sort_unstable();
        mine.dedup();
        assert_eq!(mine.len(), len, "a package twice to target {t}");
        if !bads.iter().any(|&(b, k)| b == t && k == Fails::Errors) {
            seen_new.extend(mine);
            seen_old.extend(old.0[t].iter().map(|p| number(p)));
        }
    }
    assert_eq!(seen_new, seen_old, "packages delivered");
}

#[test]
fn spreader_chain_with_a_taking_target_between_two_refusing_ones() {
    // M3R-62: the target between the two refusing ones takes some packages (its filter),
    // so a chunk can start between them and the second of the chain is written before
    // the first: its probe goes out one pass later. A debug build stopped there at an
    // assertion of the write order
    chunked_as_the_old_rule(
        &[Some("[02468]$"), Some("[0369]$"), None, None],
        &[(0, Fails::Errors), (2, Fails::Errors)],
    );
}

#[test]
fn spreader_does_not_hold_back_a_target_that_never_answered_or_closed() {
    // M3R-47, mutant 7: only a target that answered errors to all of its chunk with its
    // link kept is held back. One that never answered (or whose link went) is down, not
    // held back: when it comes back it gets its share, not one probe per chunk
    for answer in [Answer::Never, Answer::Close] {
        let (good, _good_rec) = recording_target(Answer::Int);
        let (bad, _bad_rec) = recording_target(answer);
        let (later, later_rec) = recording_target(Answer::Int);
        let c = chunk_config("spreader", &[good, bad, good]);
        let mut links: Vec<RedisLink> = c
            .clients
            .iter()
            .map(|t| new_link(1, &c, t, &None))
            .collect();
        for l in links.iter_mut() {
            connect_before_the_run(l);
        }
        let (mut counts, mut cursor) = (Counts::default(), 0);
        let packages = packages_n(3 * SHARE_CHUNK);
        let mut chunks = packages.chunks(SHARE_CHUNK);
        run_chunk(
            &c,
            &mut links,
            &mut cursor,
            chunks.next().unwrap(),
            &mut counts,
        );
        assert!(links[1].link.is_none(), "{answer:?}: the link is kept");
        assert!(!links[1].held_back, "{answer:?}: held back");
        // It comes back (another stand-in on its place), answering
        let mut back = c.clients[1].clone();
        back.port = later;
        links[1].endpoint = new_link(1, &c, &back, &None).endpoint;
        links[1].retry_at = None;
        connect_before_the_run(&mut links[1]);
        for chunk in chunks {
            run_chunk(&c, &mut links, &mut cursor, chunk, &mut counts);
        }
        let got = pushed_to(&later_rec).len();
        assert!(
            got >= 2 * SHARE_CHUNK / 3 - 1,
            "{answer:?}: {got} packages in two chunks once back"
        );
    }
}

#[test]
fn spreader_breaks_the_order_at_most_once_per_failure_event() {
    // M3R-45, plan 9.11.13 part 4 ("once per failure event"): two targets each answer
    // errors in a run of their own (two events, apart), each place of the turn; the order
    // breaks at most twice over all targets, every package delivered once
    let mut seen = 0;
    for first in 0..3 {
        let second = (first + 1) % 3;
        let mut answers = [Answer::Int; 3];
        answers[first] = Answer::ErrorIn(30, 60);
        answers[second] = Answer::ErrorIn(120, 150);
        let sent = spread_answers(&answers, &[None, None, None], 8, 30);
        // What each target kept: a stand-in records the RPUSHes it refused too
        let got: Vec<Vec<Vec<u8>>> = sent
            .iter()
            .zip(answers)
            .map(|(list, a)| {
                let refused = |p: &Vec<u8>| {
                    matches!(a, Answer::ErrorIn(from, to) if (from..to).contains(&number(p)))
                };
                list.iter().filter(|p| !refused(p)).cloned().collect()
            })
            .collect();
        let mut all = got.concat();
        all.sort();
        assert!(
            all == packages_from(0, 240),
            "{answers:?}: not every package once"
        );
        let total: usize = got.iter().map(|g| breaks(g)).sum();
        assert!(total <= 2, "{answers:?}: {total} breaks");
        seen += total;
    }
    eprintln!("\nbreaks over the three placements: {seen}");
}

// --- conclude and remove_pid: the end of a run (M3R-48 = R48a) ---

/// Runs `conclude` over two workers and the sender's slot (2), with 10 packages in and
/// out counted by worker 0; returns the exit status and the status file it wrote
fn concluded(test: &str, ending: Ending) -> (i32, serde_json::Value) {
    let dir = scratch_dir(test);
    let status = dir.join("rm.status");
    let clients = vec![client("t0", "q:t0")];
    let stats = stats_slots(3, clients.len());
    worker_slot(&stats, 0).add(&Counts {
        incoming: 10,
        outgoing: 10,
        ..Counts::default()
    });
    let mut since_start = Counts::default();
    let code = conclude(
        &ending,
        2,
        "src",
        None,
        &clients,
        &stats,
        &Some(status.to_string_lossy().into_owned()),
        get_current_time_with_ms() - 1000,
        &mut since_start,
    );
    let written = serde_json::from_slice(&fs::read(&status).unwrap()).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    (code, written)
}

#[test]
fn conclude_a_dead_thread_exits_one_with_the_abandoned_counted() {
    let (code, s) = concluded(
        "conclude_dead",
        Ending {
            died: true,
            abandoned: 7,
        },
    );
    assert_eq!(code, EXIT_FAILED);
    for key in ["total_drop", "total_failed"] {
        assert_eq!(s[key], 7, "{key}");
    }
    for key in ["drop", "failed"] {
        assert_eq!(s["since_start"][key], 7, "since_start {key}");
    }
    assert_eq!(s["since_start"]["in"], 10);
    assert_eq!(s["since_start"]["out"], 10);
}

#[test]
fn conclude_a_dead_thread_with_nothing_held_still_exits_one() {
    let (code, s) = concluded(
        "conclude_dead_empty",
        Ending {
            died: true,
            abandoned: 0,
        },
    );
    assert_eq!(code, EXIT_FAILED);
    assert_eq!(s["total_drop"], 0);
    assert_eq!(s["since_start"]["in"], 10);
}

#[test]
fn conclude_a_stop_exits_zero_with_the_counts_unchanged() {
    let (code, s) = concluded(
        "conclude_stop",
        Ending {
            died: false,
            abandoned: 0,
        },
    );
    assert_eq!(code, EXIT_OK);
    assert_eq!(s["total_drop"], 0);
    assert_eq!(s["total_failed"], 0);
    assert_eq!(s["since_start"]["out"], 10);
}

#[test]
fn remove_pid_removes_the_file_and_only_warns_when_it_is_gone() {
    let dir = scratch_dir("remove_pid");
    let pid = dir.join("rm.pid");
    fs::write(&pid, "1").unwrap();
    let pidfile = Some(pid.to_string_lossy().into_owned());
    remove_pid(&pidfile);
    assert!(!pid.exists());
    // Gone already: a warning, no panic
    remove_pid(&pidfile);
    remove_pid(&None);
    fs::remove_dir_all(&dir).unwrap();
}

// --- the error line of a failed push, rate-limited per thread and target (M3F-23) ---

#[test]
fn push_errors_that_keep_the_link_are_limited_and_counted() {
    let (mut r, mut last) = (Repeated::default(), String::new());
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    // The first at once, alone
    assert_eq!(
        limit_push_error(&mut r, &mut last, "WRONGTYPE a", true, at(0)),
        Some("WRONGTYPE a".to_string())
    );
    // Inside the window: counted, the last one kept for the end line
    for (i, e) in ["WRONGTYPE b", "OOM c", "WRONGTYPE d"].iter().enumerate() {
        assert_eq!(
            limit_push_error(&mut r, &mut last, e, true, at(1 + i as u64)),
            None
        );
    }
    assert_eq!(last, "WRONGTYPE d");
    // The first after the window: its own text and the count of all since the last line
    let due = REPEAT_WINDOW.as_millis() as u64 + 1;
    assert_eq!(
        limit_push_error(&mut r, &mut last, "OOM e", true, at(due)),
        Some("OOM e (4 since the last line)".to_string())
    );
    // Nothing counted since: no end line
    assert_eq!(r.left(&last), None);
    // One more inside the new window: the end line carries the last counted error
    assert_eq!(
        limit_push_error(&mut r, &mut last, "OOM f", true, at(due + 1)),
        None
    );
    assert_eq!(
        r.left(&last),
        Some("OOM f (1 since the last line)".to_string())
    );
}

#[test]
fn a_dropped_link_line_starts_the_window_again() {
    // M3R-61 (mutant 7): the line of a dropped link opens a new window, so an error reply
    // inside that window, even past the end of the first one, is counted, not printed
    let (mut r, mut last) = (Repeated::default(), String::new());
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let window = REPEAT_WINDOW.as_millis() as u64;
    assert!(limit_push_error(&mut r, &mut last, "WRONGTYPE a", true, at(0)).is_some());
    // The link drops at 0.9 of the first window: printed, and the window starts there
    let drop_at = window * 9 / 10;
    assert_eq!(
        limit_push_error(&mut r, &mut last, "broken pipe", false, at(drop_at)),
        Some("broken pipe".to_string())
    );
    // Past the first window, inside the new one: counted
    assert_eq!(
        limit_push_error(
            &mut r,
            &mut last,
            "WRONGTYPE b",
            true,
            at(window + window / 20)
        ),
        None
    );
    // Past the new window: printed with its count
    assert_eq!(
        limit_push_error(&mut r, &mut last, "WRONGTYPE c", true, at(drop_at + window)),
        Some("WRONGTYPE c (2 since the last line)".to_string())
    );
}

#[test]
fn a_push_error_that_drops_the_link_is_always_printed_with_the_count() {
    let (mut r, mut last) = (Repeated::default(), String::new());
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    assert!(limit_push_error(&mut r, &mut last, "WRONGTYPE a", true, at(0)).is_some());
    assert_eq!(
        limit_push_error(&mut r, &mut last, "WRONGTYPE b", true, at(1)),
        None
    );
    assert_eq!(
        limit_push_error(&mut r, &mut last, "WRONGTYPE c", true, at(2)),
        None
    );
    // Inside the window, but the link went: printed at once, with the two pending
    assert_eq!(
        limit_push_error(&mut r, &mut last, "broken pipe", false, at(3)),
        Some("broken pipe (3 since the last line)".to_string())
    );
    // Back, and dropped again a moment later: printed again, alone
    assert_eq!(
        limit_push_error(&mut r, &mut last, "connection reset", false, at(1003)),
        Some("connection reset".to_string())
    );
    // The count is the link's and survives its reconnects: an error reply after them is
    // counted in the window the last line opened
    assert_eq!(
        limit_push_error(&mut r, &mut last, "WRONGTYPE d", true, at(1004)),
        None
    );
    assert_eq!(
        r.left(&last),
        Some("WRONGTYPE d (1 since the last line)".to_string())
    );
}

// --- the link opened again without CLIENT SETNAME counts once a PING is answered (M3F-27) ---

#[test]
fn a_fallback_link_counts_once_its_ping_is_answered() {
    let io = |kind| -> redis::RedisResult<()> {
        Err(redis::RedisError::from(std::io::Error::new(kind, "x")))
    };
    // A reply, or an error reply (a user without +ping): the server is there
    assert_eq!(fallback_answered(&Ok(())), Ok(()));
    let refused: redis::RedisResult<()> =
        Err((redis::ErrorKind::ResponseError, "NOPERM no +ping").into());
    assert_eq!(fallback_answered(&refused), Ok(()));
    // Closed or reset: not there, said as such
    for kind in [
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::UnexpectedEof,
        std::io::ErrorKind::BrokenPipe,
    ] {
        let e = fallback_answered(&io(kind)).unwrap_err();
        assert!(e.starts_with("failed on PING: "), "{}", e);
    }
    // No answer within the command timeout (the socket's read timeout)
    for kind in [std::io::ErrorKind::TimedOut, std::io::ErrorKind::WouldBlock] {
        let e = fallback_answered(&io(kind)).unwrap_err();
        assert!(
            e.starts_with("did not answer PING within the command timeout"),
            "{}",
            e
        );
    }
}

#[test]
fn a_package_only_the_held_back_target_takes_is_tried_there() {
    // M3R-58: t0 answers errors to every package of the first chunk, so it is held back in
    // the second; t1's filter takes only the even packages. An odd package of the second
    // chunk is wanted by no target that is not held back: it is tried on t0 (as 1.0.8
    // would), which now takes it. The differential has no such case (the review found the
    // mutant caught only by a status test of the suite).
    let size = 10;
    let sent = spread_answers(
        &[Answer::ErrorIn(0, size), Answer::Int],
        &[None, Some("[02468]$")],
        2,
        size,
    );
    let second: Vec<Vec<u8>> = packages_n(2 * size).split_off(size);
    let (odd, even): (Vec<Vec<u8>>, Vec<Vec<u8>>) = second
        .into_iter()
        .partition(|p| (p.last().unwrap() - b'0') % 2 == 1);
    for p in &odd {
        assert!(
            sent[0].contains(p),
            "{} not tried on the held-back target: {:?}",
            String::from_utf8_lossy(p),
            sent[0]
                .iter()
                .map(|p| String::from_utf8_lossy(p))
                .collect::<Vec<_>>()
        );
    }
    // The even ones go to t1 as the rotation gives them, never to both
    for p in &even {
        assert!(
            sent[1].contains(p) != sent[0].contains(p),
            "{}",
            String::from_utf8_lossy(p)
        );
    }
}
