# RedisMultiplexer

[![CI](https://github.com/juanmitaboada/redismultiplexer/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/juanmitaboada/redismultiplexer/actions/workflows/rust.yml)
[![Quality Gate](https://sonarcloud.io/api/project_badges/measure?project=juanmitaboada_redismultiplexer&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=juanmitaboada_redismultiplexer)

This is a program to transfer data from queues (push/pop) between different Redis server

[![Get it from the Snap Store](https://snapcraft.io/static/images/badges/en/snap-store-black.svg)](https://snapcraft.io/redismultiplexer)

## Description

RedisMultiplexer will take care of your queues in Redis. The purpose is to move packages from a source queue to a target queue that may be or not be in the same server and port.

You can configure as many targets as "clients" as you would like, the system works in 2 modes (replicant and spreader):

- `replicant mode` replicates the incoming packages from the source server to all clients or destinations
- `spreader mode` will send a package to each target at a time, using a Round-Robin target selection
RedisMultiplexer will take care of your servers by checking the size of the destination queues to avoid overloading, you can control all of that with the \*limit options in your configuration, also you can filter what data is delivered where with filter\* options, finally you can reorder the incoming queue with the ordering\* options:

## Configuration

Every node or connection to Redis will shared the same `General Configuration` and optional `Filters`.

The configuration has 2 defined zones, the first one is the source server which will have all its fields on the root of the YAML configuration while the target or destination servers will be under the entry `clients`:

`Root` of the YAML configuration belongs to source server and contains:
- `General configuration`: explained below
- Optional `Filters`: explained below
- Optional `Ordering`: explained below
- `pid`: pid file of the executing RedisMultiplexer
- `status`: status file will contains a JSON string with the statistics of the program while working
- `children`: total of threads or workers to be started for processing (usually 2 is enought)
- `mode`: there 2 working modes: replicant and spreader, explained before
- `clients`: will have as many target servers as desired

Inside the `clients` entry of the YAML configuration:
- `General configuration`: explained below
- Optional `Limits`: explained below
- Optional `Filters`: explained below

### General configuration:

Every configuration will have

- `name`: will be used on the screen when RedisMultiplexer has to show some message on the screen
- `hostname`: hostname of the Redis server (an IPv6 address may be written `[::1]`)
- `port`: port number of the Redis server
- `password`: password of the Redis server; empty for none
- `channel`: name of queue to process
- `username` (optional): the Redis 6 ACL user; empty or absent for none
- `database` (optional): the database selected after connecting; 0 when absent
- `ssl` (optional): `true` connects with TLS, checking the server's certificate against the system's trusted ones; `false` (default) plain TCP
- `connect_timeout` (optional): seconds to wait for a connection; 2 when absent, 0 for no limit (the system's), otherwise at least 0.001
- `command_timeout` (optional): seconds to wait for a reply, and for a write to go out; 5 when absent, 0 for no limit, otherwise at least 0.001. The source's reads wait one second more (a read waits up to 1 s for a package)

Each connection is named with `CLIENT SETNAME redismultiplexer:<source>:<target>:<role>` (characters a name may not hold become `_`); a server that refuses the command is used without the name.

### Filters are optional:

- `filter`: this is a Regular Expression that will be matched with the header of the package
- `filter_until`: the header of the package will be as long until this substring is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_until_included`: `true` makes the `filter_until` substring part of the header, so the Regular Expression can match it and `filter_replace` replaces it too (useful to strip a routing prefix such as `1#`); `false` (default) leaves it out of the header
- `filter_limit`: the header of the package will be as long until these total bytes is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_replace`: if this filter option is defined the Regular Expression will be replaced with this string (which may contain $X groups from Regex)
- `filter_replace_header`: if this filter option is defined the whole header (not only the part the Regular Expression matched) is replaced with this literal string, without $X groups; an empty string removes the header. Use either `filter_replace` or `filter_replace_header`, not both

Filters work on bytes: packages do not need to be valid UTF-8, and `filter_limit` counts bytes.

As an example:
- For the incoming string "Hello world"
- We would like the filter to stop analizying after 11 bytes or "r"
- We would like to filter "ell"
- We would like the filtered string being replaced with ELL, so the result would e "HELLo world"
- We would use the configuration:
```yaml
filter: "ell"
filter_until: "r"
filter_limit: 11
filter_replace: "ELL"
```

### Limits are optional:

- `timelimit`: the size of the queue will be checked every n-seconds
- `checklimit`: the size of the queue will be checked every n-packages
- `hardlimit`: the software will `stop` sending packages to the destination until the queue is freed and `softlimit` is reached
- `softlimit`: the software will `continue` sending packages to the destination after a `hardlimit` was detected and the queue is freed until being under `softlimit`
- `deleteblock`: when `hardlimit` is reached the software will delete n-oldests-packages from the queue as many times until the size of the queue is under `hardlimit`

`hardlimit` turns the limits on; the other keys need it. Without them: `softlimit` is 80 % of `hardlimit` (at least 1), `timelimit` is 1 and `checklimit` is 10000, and the queue is checked at whichever comes first. A key set to 0 is not set; `deleteblock: 0` too (a warning at start says so). Each worker checks the queue on its own, so between two checks the queue can pass `hardlimit` by up to `children` x `checklimit` packages (with ordering, one x `checklimit`: only the sender delivers); for a small `hardlimit`, set `checklimit` too.

### Ordering is optional:

When several servers are dumping their information to the same queue the packages maybe disordered since the casuality of the real-time procesing. Let's imagine that you need to process this data by a Machine Learning system and because it is unordered the ML will may learn the future of its actions. It would be very easy to avoid this problem if the Queue gets ordered before being processed.

This feature take care of ordering the incoming queue before being processed, it keeps a timed buffer of few seconds and use a prefix and postfix to find a string that can be parsed to be used for comparing with other packages. RedisMultiplexer will extract the first `ordering_limit` bytes from the package and it will look for `ordering` Regular Expression matching the named group "ts" as the timestamp used for ordering the list of packages. The ts string extracted will be parsed as u128:

- `ordering`: what is the Regular Expression used to parse the times substring (matching with group 'ts' will be required in your Regex expression. Example:
```yaml
ordering: '.*"ts": *(?P<ts>\d+),.*#'
``` 
- `ordering_buffer_time`: total of seconds that new packages must stay in the buffer so they get ordered. 0 is read as 1, as earlier versions held such a package up to a second (a warning at start says so); without the key, packages are released at once and only those that arrive together are sorted
- `ordering_limit`: how many bytes to process during the extraction of the substring

As an example:
- For the package: '{"a": "abc", "ts": 12345678, "b": 88}'
- Where "ts" is the key holding the timed string, example:
```yaml
ordering: '.*"ts": *(?P<ts>\d+),.*#'
ordering_buffer_time: 30
ordering_limit: 200
```

What ordering guarantees:
- A package is held `ordering_buffer_time` seconds from its arrival, then released in `ts` order (equal `ts` in arrival order). A held package with a lower `ts` holds back the ones behind it, so the order holds among the packages the buffer has at once: one that arrives after a later `ts` was released goes out after it.
- A package without a match, or whose `ts` is not a number that fits u128, is delivered at once, without waiting, and a warning says so (see Log).
- With ordering, one sender thread delivers for every child. It writes the released packages to each target in chunks (up to 1000 packages or 4 MiB, a larger package alone), pipelined, and each target gets its packages in release order.
- In `spreader` mode the packages are shared as the round robin would share them one by one. When a target refuses in its replies (an error, no reply, the connection lost), what it refused goes to another target. The order at that target can then break once per failure event, by at most the packages refused in that chunk, and the shares can differ from the round robin's in that chunk. A target that answers errors to every package of a chunk is held back: it is offered one package per chunk until it takes one, and it is back to its share at most one chunk later.

## How all of this works

### Example 1: forwarding packages between server

RedisMultiplexer can be used as a forwarder between server. This is specially good to transfer packages between servers and avoid overloading of the source server. It is a very good idea to set RedisMultiplexer on the source side of the connection so if there is some network delays or outages, the system will be dropping the packages on the source avoiding problems derived from source queue being overloaded.

### Example 2: load balancing queues between several servers

RedisMultiplexer may be used as a load balancer. Using the "spreader" mode you can put packages into the source queue and RedisMultiplexer will spread all the packages to all the target servers or `clients`. When some of the servers get overloaded RedisMultiplexer will avoid it for a while until its queue is freed.

### Example 3: replicating queues into several servers

RedisMultiplexer may be used as a replicant system. Using the "replicant" mode you can put packages into the source queue and RedisMultiplexer will send a copy of it to all to all the target servers or `clients`. When some of the servers get overloaded RedisMultiplexer will avoid it for a while until its queue is freed.


### Example 4: retention system for network outages

One of the interesting uses of RedisMultiplexer is as a retention system for network delays and network outages. This is very good when it is used on the source side of the connection, because it will keep the local queue empty if there are network problems or overloaded queues on the remote side of the connection.

### Example 5: multiple RedisMultiplexer services

It is a common use to set up several RedisMultiplexer in the same server. Les's imagine you would like to teach several ML systems on remote servers and each of them will be teached with specific data depending on the incoming packages.

Most probably the best approach would be to set up a RedisMultiplexer in `replicant` mode to split in the same way to all clients and filter the data in several queues (as many as different ML systems) we will have, the filtering system per client will help you to decide what data is sent to what queue. You may use ordering system as well to keep order of the incoming data.

Now let's imagine that one of the queues will be used to teach the same ML with different setups, so then you would use a RedisMultiplexer with the `replicant` mode to send data to both ML systems.

But let's go farther, you will need a test data and a learning data in disjoint groups so learning system won't learn from test data and we can use test data for prediction to test how good the ML system is learning. You can split data in percentages, lets say 10% test / 90% learning. You would use a RedisMultiplexer in `spreader` mode to 10 different clients, 1 of those clients will be the testing queue while the other 9 clients will be the same learning queue. Still you may would like another RedisMultiplexer as a forwarder to reorder data.

Each of this queues you just made are still in the same server, so you would use one RedisMultiplexer by each of those queues to put data out from that server to another remote server, in this way you would be using RedisMultiplexer as a rentention system in case there are network issues.

## Running

```sh
redismultiplexer <path_to_config.yaml>
redismultiplexer <name> systemd     # /etc/redismultiplexer/<name>.yaml, pid and status in /run/redismultiplexer/
redismultiplexer --version          # also: version
redismultiplexer --help             # also: help, -h
```

At start the configuration is checked. A file that cannot be used (an unknown `mode`, no clients, a limit key without `hardlimit`, a filter option without `filter`, a timeout out of range, a regular expression that does not compile, among others) stops the program with exit code 2 and the reason. A key the program does not know is named in a warning and ignored; settings that work but are odd (`ordering_buffer_time` or `ordering_limit` without `ordering`, `ordering` without `ordering_buffer_time`, `ordering_buffer_time: 0`, `deleteblock: 0`, `softlimit` not under `hardlimit`, a client name empty or used twice) get a warning too, and the program starts.

Signals: SIGINT, SIGTERM and SIGHUP stop the program cleanly. Started with SIGHUP ignored (as under `nohup`), it keeps ignoring it.

The stop: each child finishes the package it holds (a read from the source waits at most a second), then, with ordering, the sender delivers everything the buffer holds, in order, and the pid file is removed. With a target that never answers, the stop takes at most one `command_timeout` for the chunk in flight, plus the delivery of what the buffer holds at the pipelined rate, plus one `command_timeout` for each reconnection to that target within that time: there is no bound that does not depend on what the buffer holds.

Exit codes:
- `0`: stopped by a signal, `--version`, `--help`
- `1`: a thread ended on its own (a child or the sender); the others are stopped and what the buffer held is counted as dropped
- `2`: the configuration cannot be read or used, or no argument was given

## Status file

With `status` set (or `systemd`), a JSON object is written every 10 seconds and at the stop. The file is replaced whole: the text goes to `<status>.tmp` beside it and is renamed over it, so a reader never sees half of it. A status path that is a symbolic link stays a link (the file it points to is replaced), and an existing file keeps its mode, owner and group. Where the file cannot be replaced (a directory the program may not write, for one), it is written in place, with one warning the first time.

| Key | Meaning |
|---|---|
| `date` | when it was written (seconds since the epoch) |
| `in`, `out`, `drop`, `deleted` | packages per second over the last interval: read from the source, delivered, dropped, deleted from target queues by `deleteblock` |
| `total_in`, `total_out`, `total_drop`, `total_deleted` | the same, as counts over the last interval |
| `filtered`, `stuck`, `failed` | the breakdown of `drop`, per second: no target accepted the package (the filters); every target that accepted it was stuck at its `hardlimit`; a target that accepted it could not take it (down, an error, no reply) |
| `total_filtered`, `total_stuck`, `total_failed` | the same, as counts; they add up to `total_drop` |
| `queue` | packages held in the ordering buffer (0 without ordering) |
| `stuck_targets`, `down_targets` | `<name>:<channel>` of the targets stuck at their `hardlimit`, and of those without a connection |
| `since_start` | `in`, `out`, `drop`, `deleted`, `filtered`, `stuck`, `failed` as counts since the start |
| `version` | the program's version |

A package that was not delivered counts once in `drop`: `failed` when a target that accepted it could not take it, else `stuck` when every target that accepted it was stuck, else `filtered`.

## Log

Each line is `<date> RedisMultiplexer: <message>`, written whole in one call. The banner, the start-up summary and a statistics line every 10 seconds go to stdout; errors and warnings go to stderr. Colours are used only when the stream is a terminal. A line that cannot be written (a closed pipe, a full disk) is dropped and the program goes on.

A warning that can come once per package (a package that cannot be ordered) is printed the first time, then at most once every 10 seconds per thread and kind, as `<warning> (N since the last line)`, where N counts the packages since the line before; when the thread ends, the ones not printed yet get one last such line.

## When a target fails

A target that cannot be reached, closes its connection or does not answer within `command_timeout` is down: the threads go on without it and try to connect again in the background, after 1 second and then up to every 5 seconds. Packages are not kept for a target that is down. In `replicant` mode the other targets still get them; in `spreader` mode they go to the next target; a package no target could take is dropped and counted as `failed`. A target that answers an error keeps its connection, and the package it refused is handled the same way. A target stuck at its `hardlimit` is skipped the same way until its queue is under `softlimit` (or trimmed by `deleteblock`), and a package only stuck targets accepted counts as `stuck`.

## Testing and benchmarking

Unit tests live in `src/tests.rs`; integration tests under `tests/` start the
real binary against a Redis server given by `RM_TEST_REDIS_URL` (default
`redis://127.0.0.1:6379`, database 0) and use keys under `rmtest:`. They run in
release and in debug builds (debug has the overflow checks on):

```sh
redis-server --port 6390 --save '' --appendonly no &
RM_TEST_REDIS_URL=redis://127.0.0.1:6390 cargo test --release
RM_TEST_REDIS_URL=redis://127.0.0.1:6390 cargo test
```

`bench/` holds a black-box throughput bench (Python 3.11+, standard library
only) driven by a scenario matrix, with keys under `rmbench:`:

```sh
python3 bench/rmbench.py run --tag quick --label local \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results /tmp/rmbench
```

See `bench/README.md` for scenarios, metrics and the renderer contract, and
`bench/BASELINE.md` for the reference numbers.

## Full example configuration

```yaml
name        : "Source"
hostname    : "127.0.0.1"
port        : 6379
password    : "abcdefghijklmnopqrstuvwxyz"
channel     : "SourceQueue"
children    : 2
mode        : "replicant"               # choose between: "replicant" and "spreader"
pid         : "config.pid"              # optional
status      : "config.stat"             # optional
filter      : "ell"                     # optional
filter_until: "r"                       # optional
filter_limit: 11                        # optional
filter_replace: "ELL"                   # optional
ordering: '.*"ts": *(?P<ts>\d+),.*#'    # optional
ordering_buffer_time: 30                # optional
ordering_limit: 200                     # optional
username    : "app"                     # optional
database    : 0                         # optional
ssl         : false                     # optional
connect_timeout: 2                      # optional
command_timeout: 5                      # optional

clients:
  - name        : "Target 1"
    hostname    : "127.0.0.1"
    port        : 6379
    password    : "abcdefghijklmnopqrstuvwxyz"
    channel     : "TargetQueue1"
    timelimit   : 5                     # optional
    checklimit  : 100                   # optional
    softlimit   : 400                   # optional
    hardlimit   : 410                   # optional
    deleteblock : 100                   # optional
    filter      : "^(1|3|5|7|9)#"       # optional
    filter_until: "#"                   # optional
    filter_until_included: true         # optional
    filter_limit: 100                   # optional
    filter_replace: ""                  # optional
    ssl         : true                  # optional
    command_timeout: 1.5                # optional
  - name        : "DB2"
    hostname    : "127.0.0.1"
    port        : 6379
    password    : "abcdefghijklmnopqrstuvwxyz"
    channel     : "TargetQueue2"
    timelimit   : 5                     # optional
    checklimit  : 100                   # optional
    softlimit   : 400                   # optional
    hardlimit   : 410                   # optional
    filter      : "^(0|2|4|6|8)#"       # optional
    filter_until: "#"                   # optional
    filter_until_included: true         # optional
    filter_limit: 100                   # optional
    filter_replace: ""                  # optional
```

## License

Copyright 2022-2026 Juanmi Taboada. Licensed under the Apache License 2.0 (LICENSE).
