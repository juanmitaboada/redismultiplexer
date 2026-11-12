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
    let c = parse_yaml(&body[begin..end]).unwrap();
    assert_eq!(
        (c.name.as_str(), c.children, c.mode.as_str()),
        ("Source", 2, "replicant")
    );
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
    assert!(err(c).contains("filtering option but filter is not defined"));
    let mut c = config();
    c.filter_until_included = Some(false);
    assert!(err(c).contains("filtering option but filter is not defined"));
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
    assert!(err(c).contains("filtering option but filter is not defined"));
    let mut c = config();
    c.filter_replace_header = Some("x".into());
    assert!(err(c).contains("filtering option but filter is not defined"));
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
            send_ordered(&b, hold, |batch| sink.lock().unwrap().extend(batch));
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
fn ordered_sender_waits_without_spinning_and_wakes_on_push() {
    // Nothing held: the sender waits on the condition variable (no periodic wake-up) and a
    // push into the empty buffer wakes it.
    let buffer = Arc::new(OrderBuffer::new());
    let out = Arc::new(Mutex::new(Vec::new()));
    let (b, sink) = (buffer.clone(), out.clone());
    let sender = thread::spawn(move || {
        send_ordered(&b, Duration::ZERO, |batch| {
            sink.lock().unwrap().extend(batch)
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
fn verify_rejects_deleteblock_zero() {
    // M3F-4: with the target over hardlimit, LTRIM 0 removes nothing and the loop never ends
    let mut c = config();
    c.clients[0].timelimit = Some(1);
    c.clients[0].checklimit = Some(1);
    c.clients[0].softlimit = Some(5);
    c.clients[0].hardlimit = Some(10);
    c.clients[0].deleteblock = Some(0);
    assert!(err(c).contains("deleteblock: 0"));
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
fn verify_accepts_empty_client_name_current_behaviour() {
    // M1F-6: the client hostname is checked twice and the name never.
    let mut c = config();
    c.clients[0].name = String::new();
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
            worker_slot(&stats, id).add(k, 10 * k, 100 * k, 1000 * k);
            worker_slot(&stats, id).stuck[(id % 2) as usize].store(true, Ordering::Relaxed);
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
            assert!(w.stuck[i % 2].load(Ordering::Relaxed), "slot {}", i);
            assert!(!w.stuck[1 - i % 2].load(Ordering::Relaxed), "slot {}", i);
        }
    }
}

#[test]
fn stats_are_summed_and_reset_per_interval() {
    let stats = stats_slots(3, 3);
    stats[0].add(1, 2, 3, 4);
    stats[2].add(10, 20, 30, 40);
    stats[1].stuck[2].store(true, Ordering::Relaxed);
    assert_eq!(take_counts(&stats), (11, 22, 33, 44));
    assert_eq!(take_counts(&stats), (0, 0, 0, 0));
    assert_eq!(stuck_clients(&stats, 3), vec![2]);
    stats[1].stuck[2].store(false, Ordering::Relaxed);
    assert!(stuck_clients(&stats, 3).is_empty());
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
fn old_spread(order: &mut Vec<usize>, mut attempt: impl FnMut(usize) -> bool) -> Option<usize> {
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
