//! Root and client filters.

mod common;

use common::*;
use std::thread;
use std::time::Duration;

#[test]
fn root_filter_until_limit_replace() {
    // README example: filter "ell", until "r", limit 11, replace "ELL".
    let mut ctx = Ctx::new("root_filter");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let input: Vec<String> = [
        "Hello world",   // match before "r" and within 11 bytes
        "Goodbye",       // no match: dropped
        "Hi there hell", // "ell" only after the until delimiter: dropped
        "xxxxxxxxxxell", // "ell" crosses the 11-byte limit: dropped
        "Shell",         // no delimiter, match: replaced
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    ctx.push(&src, &input);
    let extra = "filter: 'ell'\nfilter_until: 'r'\nfilter_limit: 11\nfilter_replace: 'ELL'\n";
    let cfg = ctx.config(
        &src,
        1,
        "replicant",
        extra,
        &[ctx.client_yaml("t0", &t0, "")],
    );
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&t0, 2)]);
    thread::sleep(Duration::from_millis(300));
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&t0), vec!["HELLo world", "ShELL"]);
}

#[test]
fn client_filter_with_replace() {
    let mut ctx = Ctx::new("client_filter");
    let src = ctx.key("src");
    let even = ctx.key("even");
    let odd = ctx.key("odd");
    let witness = ctx.key("witness");
    let input: Vec<String> = ["even:2 x", "odd:3 y", "none:4 z"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    ctx.push(&src, &input);
    let clients = [
        ctx.client_yaml(
            "even",
            &even,
            "    filter: '^even:(\\d+)'\n    filter_replace: 'E$1'\n",
        ),
        ctx.client_yaml("odd", &odd, "    filter: '^odd:'\n"),
        ctx.client_yaml("witness", &witness, ""),
    ];
    let cfg = ctx.config(&src, 1, "replicant", "", &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&witness, 3)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.list(&even), vec!["E2 x"]);
    assert_eq!(ctx.list(&odd), vec!["odd:3 y"]);
    assert_eq!(ctx.list(&witness), input);
}

#[test]
fn readme_client_filter_example_never_matches_current_behaviour() {
    // A-C3: filter_until removes the delimiter from the haystack, so the README
    // client example `^(1|3|5|7|9)#` with until "#" never matches. M3 decides
    // whether the behaviour or the README changes.
    let mut ctx = Ctx::new("readme_filter");
    let src = ctx.key("src");
    let t0 = ctx.key("t0");
    let witness = ctx.key("witness");
    let input: Vec<String> = ["1#a", "2#b", "3#c"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    ctx.push(&src, &input);
    let filter = "    filter: '^(1|3|5|7|9)#'\n    filter_until: '#'\n    filter_limit: 100\n    filter_replace: ''\n";
    let clients = [
        ctx.client_yaml("t0", &t0, filter),
        ctx.client_yaml("witness", &witness, ""),
    ];
    let cfg = ctx.config(&src, 1, "replicant", "", &clients);
    let mut p = ctx.spawn(&cfg);
    ctx.wait_drained(&src, &[(&witness, 3)]);
    p.stop(&ctx.pid_file());
    assert_eq!(ctx.llen(&t0), 0);
}
