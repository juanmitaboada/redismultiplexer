# Bench baseline

Reference numbers for redismultiplexer 1.0.7, measured with
`bench/rmbench.py` before any performance work. Later versions are compared
against them with `rmbench.py compare`.

## Environment

| Item | Value |
|---|---|
| CPU | 12th Gen Intel Core i7-1250U, 12 logical CPUs, 30 GiB RAM |
| OS | Ubuntu 26.04.1 LTS, kernel 7.0.0-42-generic |
| Redis | 8.0.5, local, 127.0.0.1:6390, no persistence |
| Binary | 1.0.7 program code, `cargo build --locked --release`, rustc 1.99.1 |
| Harness | Python 3.14.4 |
| Date | 2026-11-10 |

## How it was measured

```sh
python3 bench/rmbench.py run --tag quick --repeat 3 --label rust-1.0.7 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/1.0.7
python3 bench/rmbench.py run --tag full --label rust-1.0.7 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/1.0.7
```

Every run ended `ok` with 0 missing packages.

## quick (median of 3)

| scenario | status | msg/s | MiB/s | drain s | stop s | missing | cpu user s | cpu sys s | max RSS MiB |
|---|---|---|---|---|---|---|---|---|---|
| q_base.c1 | ok | 914 | 0.17 | 5.47 | 2.57 | 0 | 0.14 | 0.27 | 6.2 |
| q_base.c2 | ok | 1824 | 0.35 | 2.74 | 3.27 | 0 | 0.11 | 0.21 | 6.4 |
| q_base.c8 | ok | 2935 | 0.56 | 1.70 | 3.34 | 0 | 0.11 | 0.20 | 6.9 |
| q_fanout | ok | 1826 | 0.35 | 2.74 | 3.28 | 0 | 0.12 | 0.19 | 6.4 |
| q_limits | ok | 1834 | 0.35 | 2.73 | 3.29 | 0 | 0.10 | 0.20 | 6.4 |
| q_payload4k | ok | 1825 | 7.13 | 2.74 | 3.28 | 0 | 0.13 | 0.20 | 6.4 |
| q_ordering | ok | 1031 | 0.20 | 4.85 | 3.16 | 0 | 0.15 | 0.23 | 8.1 |
| q_filter.froot | ok | 1810 | 0.17 | 2.76 | 3.27 | 0 | 0.16 | 0.17 | 7.0 |
| q_filter.fclient | ok | 1830 | 0.17 | 2.73 | 3.28 | 0 | 0.15 | 0.20 | 7.1 |
| q_mixed_plain.i0 | ok | 1851 | 0.35 | 2.70 | 3.30 | 0 | 0.10 | 0.18 | 6.3 |
| q_mixed_plain.i1 | ok | 1848 | 0.35 | 2.71 | 4.00 | 0 | 0.13 | 0.24 | 6.3 |
| q_mixed_plain.i2 | ok | 1852 | 0.35 | 2.70 | 4.00 | 0 | 0.16 | 0.30 | 6.5 |
| q_mixed_plain | ok | 5533 | 1.05 | 2.71 | 4.01 | 0 | 0.40 | 0.72 | 6.5 |
| q_mixed.i0 | ok | 1855 | 0.35 | 2.69 | 3.23 | 0 | 0.10 | 0.22 | 6.4 |
| q_mixed.i1 | ok | 1854 | 0.35 | 2.70 | 4.00 | 0 | 0.14 | 0.24 | 6.3 |
| q_mixed.i2 | ok | 1044 | 0.20 | 4.79 | 4.00 | 0 | 0.16 | 0.32 | 8.3 |
| q_mixed.i3 | ok | 1855 | 0.18 | 2.70 | 4.01 | 0 | 0.32 | 0.31 | 7.0 |
| q_mixed | ok | 4175 | 0.70 | 4.79 | 4.01 | 0 | 0.71 | 1.10 | 8.3 |

## full

| scenario | status | msg/s | MiB/s | drain s | stop s | missing | cpu user s | cpu sys s | max RSS MiB |
|---|---|---|---|---|---|---|---|---|---|
| payload.p64.c1 | ok | 916 | 0.06 | 21.84 | 3.18 | 0 | 0.52 | 0.96 | 6.4 |
| payload.p64.c2 | ok | 1839 | 0.11 | 10.87 | 3.14 | 0 | 0.38 | 0.71 | 6.3 |
| payload.p64.c4 | ok | 3095 | 0.19 | 6.46 | 3.57 | 0 | 0.39 | 0.77 | 6.7 |
| payload.p64.c8 | ok | 5334 | 0.33 | 3.75 | 3.29 | 0 | 0.33 | 0.58 | 7.1 |
| payload.p200.c1 | ok | 919 | 0.17 | 21.77 | 3.25 | 0 | 0.48 | 1.00 | 6.3 |
| payload.p200.c2 | ok | 1834 | 0.35 | 10.90 | 3.12 | 0 | 0.38 | 0.62 | 6.4 |
| payload.p200.c4 | ok | 3685 | 0.70 | 5.43 | 2.59 | 0 | 0.37 | 0.57 | 6.7 |
| payload.p200.c8 | ok | 7294 | 1.39 | 2.74 | 3.29 | 0 | 0.29 | 0.63 | 7.1 |
| payload.p4096.c1 | ok | 914 | 3.57 | 21.89 | 3.14 | 0 | 0.59 | 0.96 | 6.3 |
| payload.p4096.c2 | ok | 1835 | 7.17 | 10.90 | 3.12 | 0 | 0.40 | 0.68 | 6.5 |
| payload.p4096.c4 | ok | 3670 | 14.34 | 5.45 | 2.58 | 0 | 0.42 | 0.67 | 6.8 |
| payload.p4096.c8 | ok | 7260 | 28.36 | 2.75 | 3.28 | 0 | 0.35 | 0.58 | 7.2 |
| payload64k.c1 | ok | 910 | 56.89 | 2.20 | 2.82 | 0 | 0.16 | 0.17 | 6.5 |
| payload64k.c2 | ok | 1825 | 114.06 | 1.10 | 2.92 | 0 | 0.13 | 0.17 | 7.4 |
| payload64k.c4 | ok | 1270 | 79.34 | 1.57 | 3.45 | 0 | 0.17 | 0.17 | 8.1 |
| payload64k.c8 | ok | 1547 | 96.71 | 1.29 | 2.74 | 0 | 0.24 | 0.21 | 10.4 |
| fanout.c1.mreplicant.n1 | ok | 919 | 0.17 | 21.76 | 3.27 | 0 | 0.50 | 0.94 | 6.4 |
| fanout.c1.mreplicant.n3 | ok | 917 | 0.53 | 21.80 | 3.23 | 0 | 0.60 | 1.22 | 6.5 |
| fanout.c1.mspreader.n1 | ok | 921 | 0.18 | 21.72 | 3.30 | 0 | 0.50 | 0.88 | 6.3 |
| fanout.c1.mspreader.n3 | ok | 914 | 0.17 | 21.87 | 3.15 | 0 | 0.52 | 0.93 | 6.4 |
| fanout.c4.mreplicant.n1 | ok | 3103 | 0.59 | 6.45 | 2.57 | 0 | 0.30 | 0.58 | 6.5 |
| fanout.c4.mreplicant.n3 | ok | 3670 | 2.10 | 5.45 | 2.57 | 0 | 0.50 | 1.04 | 6.8 |
| fanout.c4.mspreader.n1 | ok | 3106 | 0.59 | 6.44 | 2.58 | 0 | 0.30 | 0.60 | 6.6 |
| fanout.c4.mspreader.n3 | ok | 3099 | 0.59 | 6.45 | 2.56 | 0 | 0.33 | 0.60 | 6.7 |
| fanout.c8.mreplicant.n1 | ok | 5357 | 1.02 | 3.73 | 3.30 | 0 | 0.33 | 0.79 | 7.0 |
| fanout.c8.mreplicant.n3 | ok | 5309 | 3.04 | 3.77 | 3.28 | 0 | 0.48 | 1.12 | 7.5 |
| fanout.c8.mspreader.n1 | ok | 5324 | 1.02 | 3.76 | 3.28 | 0 | 0.27 | 0.57 | 6.9 |
| fanout.c8.mspreader.n3 | ok | 5343 | 1.02 | 3.74 | 3.29 | 0 | 0.29 | 0.56 | 7.0 |
| limits.n1 | ok | 1847 | 0.35 | 10.83 | 3.18 | 0 | 0.36 | 0.73 | 6.6 |
| limits.n3 | ok | 1843 | 1.05 | 10.85 | 3.17 | 0 | 0.48 | 1.12 | 6.5 |
| limits_time | ok | 1838 | 0.35 | 10.88 | 3.14 | 0 | 0.43 | 0.84 | 6.5 |
| ordering.c1 | ok | 825 | 0.16 | 24.24 | 2.78 | 0 | 0.56 | 0.97 | 7.8 |
| ordering.c2 | ok | 1502 | 0.29 | 13.32 | 2.70 | 0 | 0.44 | 0.72 | 8.8 |
| ordering.c8 | ok | 4060 | 0.77 | 4.93 | 3.10 | 0 | 0.36 | 0.63 | 12.1 |
| filter.c1.froot | ok | 919 | 0.09 | 21.77 | 3.26 | 0 | 0.76 | 0.80 | 6.9 |
| filter.c1.fclient | ok | 915 | 0.09 | 21.86 | 3.16 | 0 | 0.84 | 0.79 | 7.0 |
| filter.c2.froot | ok | 1828 | 0.17 | 10.94 | 3.08 | 0 | 0.64 | 0.63 | 7.2 |
| filter.c2.fclient | ok | 1846 | 0.18 | 10.84 | 3.18 | 0 | 0.68 | 0.57 | 7.2 |
| filter.c8.froot | ok | 7321 | 0.70 | 2.73 | 3.30 | 0 | 0.55 | 0.43 | 7.9 |
| filter.c8.fclient | ok | 7336 | 0.70 | 2.73 | 3.31 | 0 | 0.64 | 0.46 | 8.2 |
| mixed_plain.i0 | ok | 1856 | 0.35 | 10.78 | 3.24 | 0 | 0.31 | 0.65 | 6.4 |
| mixed_plain.i1 | ok | 1857 | 0.35 | 10.77 | 4.01 | 0 | 0.39 | 0.69 | 6.5 |
| mixed_plain.i2 | ok | 1856 | 0.35 | 10.78 | 4.01 | 0 | 0.43 | 0.77 | 6.4 |
| mixed_plain | ok | 5566 | 1.06 | 10.78 | 4.01 | 0 | 1.13 | 2.11 | 6.5 |
| mixed.i0 | ok | 1859 | 0.35 | 10.76 | 3.85 | 0 | 0.35 | 0.67 | 6.4 |
| mixed.i1 | ok | 1861 | 0.35 | 10.75 | 3.98 | 0 | 0.36 | 0.70 | 6.5 |
| mixed.i2 | ok | 1517 | 0.29 | 13.18 | 3.00 | 0 | 0.41 | 0.67 | 8.5 |
| mixed.i3 | ok | 1861 | 0.18 | 10.74 | 4.01 | 0 | 0.72 | 0.65 | 7.1 |
| mixed | ok | 6068 | 1.01 | 13.18 | 4.01 | 0 | 1.85 | 2.69 | 8.5 |

## Notes

- 1.0.7 is capped at about 915 msg/s per worker thread (the ordering queuer
  polls with a 1 ms sleep), whatever the payload, mode or number of clients;
  throughput grows roughly linearly with `children`.
- `q_base.c8` (5000 packages) is dominated by start-up; use
  `payload.p200.c8` for the 8-children figure. `payload64k` uses 2000
  packages, so its 4 and 8 children runs are too short to trust.
- Stop time after SIGINT is 2.6-4.0 s (fixed sleeps in the shutdown path); it
  is not part of the throughput figures. Peak RSS is 6-12 MiB.
- The harness used about 1.2 % of one CPU while the runs were draining.
- A first quick pass overlapped other work on the laptop and was discarded;
  the table above is the repeated pass (within 1-2 % of the first).

# 1.0.8

Release 1.0.8: the queuer waits on its channel instead of polling every
millisecond, and each worker reads its own queuer replies. 1.0.7 and 1.0.8
were measured on the same day, on the same machine, with the same harness
(the one that adds the time-boxed `fast` tier), so the tables below compare
like with like. The 1.0.7 tables above were taken with the earlier harness and
counts and stay as they were.

## Environment

As for 1.0.7 above (same laptop, OS, kernel, Redis 8.0.5 on 127.0.0.1:6390,
rustc 1.99.1, Python 3.14.4). Binaries: 1.0.7 from commit 76deefc and 1.0.8
from the `release: 1.0.8` commit, both `cargo build --locked --release`.
The later commit `fix: read the stop order before answering a queuer request`
only moves the queuer's read of its stop order ahead of each answer, with the
same work per request, so it does not change the path measured here.
Date: 2026-11-10.

## How it was measured

```sh
# fast: time-boxed, 5 s per run, about 2 min per build for 3 repetitions
python3 bench/rmbench.py run --tag fast --repeat 3 --label rust-1.0.8 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/fast/1.0.8
# deep: quick matrix with 30 times the counts (150000 packages per source),
# 49 min for 1.0.7 and 5 min for 1.0.8
python3 bench/rmbench.py run --tag quick --repeat 3 --scale 30 \
    --max-bytes 2147483648 --timeout 600 --label rust-1.0.8 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/deep/1.0.8
# q_base.c8 again at --scale 60: at scale 30 1.0.8 drains it in 1.6 s
python3 bench/rmbench.py run --scenario q_base.c8 --repeat 3 --scale 60 \
    --max-bytes 2147483648 --timeout 600 --label rust-1.0.8 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/deep-c8/1.0.8
```

The same commands with `--label rust-1.0.7` and the 1.0.7 binary give the
1.0.7 columns. Every run of both builds ended `ok` with 0 missing packages
(none of these runs stops a busy source with ordering and one child, the case
that fix is about).

## fast (median of 3, msg/s)

| scenario | rust-1.0.7 msg/s | rust-1.0.8 msg/s | ratio rust-1.0.8/rust-1.0.7 | rust-1.0.7 missing | rust-1.0.8 missing |
|---|---|---|---|---|---|
| fast.c1 | 918 | 29557 | 32.20 | 0 | 0 |
| fast.c2 | 1836 | 38264 | 20.84 | 0 | 0 |
| fast.c8 | 7333 | 86405 | 11.78 | 0 | 0 |
| fast_fanout | 1844 | 38292 | 20.76 | 0 | 0 |
| fast_ordering | 1776 | 39722 | 22.36 | 0 | 0 |

1.0.8 detail:

| scenario | status | msg/s | MiB/s | drain s | stop s | missing | cpu user s | cpu sys s | max RSS MiB |
|---|---|---|---|---|---|---|---|---|---|
| fast.c1 | ok | 29557 | 5.64 | 5.01 | 3.02 | 0 | 1.04 | 2.37 | 7.0 |
| fast.c2 | ok | 38264 | 7.30 | 5.01 | 3.01 | 0 | 2.06 | 4.31 | 7.2 |
| fast.c8 | ok | 86405 | 16.48 | 5.01 | 2.02 | 0 | 4.65 | 9.45 | 9.2 |
| fast_fanout | ok | 38292 | 7.30 | 5.00 | 2.03 | 0 | 1.78 | 3.64 | 7.4 |
| fast_ordering | ok | 39722 | 5.60 | 5.00 | 3.02 | 0 | 2.13 | 3.75 | 40.1 |

## deep: quick at scale 30 (median of 3, msg/s)

| scenario | rust-1.0.7 msg/s | rust-1.0.8 msg/s | ratio rust-1.0.8/rust-1.0.7 | rust-1.0.7 missing | rust-1.0.8 missing |
|---|---|---|---|---|---|
| q_base.c1 | 922 | 30295 | 32.86 | 0 | 0 |
| q_base.c2 | 1838 | 38316 | 20.84 | 0 | 0 |
| q_base.c8 (scale 60) | 7150 | 91165 | 12.75 | 0 | 0 |
| q_fanout | 1831 | 38642 | 21.10 | 0 | 0 |
| q_limits | 1833 | 38887 | 21.21 | 0 | 0 |
| q_payload4k | 1836 | 32604 | 17.76 | 0 | 0 |
| q_ordering | 1727 | 29590 | 17.13 | 0 | 0 |
| q_filter.froot | 1834 | 40920 | 22.31 | 0 | 0 |
| q_filter.fclient | 1832 | 38926 | 21.24 | 0 | 0 |
| q_mixed_plain.i0 | 1854 | 28994 | 15.63 | 0 | 0 |
| q_mixed_plain.i1 | 1856 | 28658 | 15.44 | 0 | 0 |
| q_mixed_plain.i2 | 1855 | 28440 | 15.33 | 0 | 0 |
| q_mixed_plain | 5562 | 85306 | 15.34 | 0 | 0 |
| q_mixed.i0 | 1859 | 23906 | 12.86 | 0 | 0 |
| q_mixed.i1 | 1860 | 23909 | 12.86 | 0 | 0 |
| q_mixed.i2 | 1752 | 17972 | 10.26 | 0 | 0 |
| q_mixed.i3 | 1860 | 26349 | 14.17 | 0 | 0 |
| q_mixed | 7009 | 71882 | 10.26 | 0 | 0 |

1.0.8 detail:

| scenario | status | msg/s | MiB/s | drain s | stop s | missing | cpu user s | cpu sys s | max RSS MiB |
|---|---|---|---|---|---|---|---|---|---|
| q_base.c1 | ok | 30295 | 5.78 | 4.95 | 3.06 | 0 | 0.88 | 1.94 | 6.8 |
| q_base.c2 | ok | 38316 | 7.31 | 3.92 | 3.11 | 0 | 1.36 | 2.84 | 7.2 |
| q_base.c8 (scale 60) | ok | 91165 | 17.39 | 3.29 | 2.74 | 0 | 3.13 | 6.33 | 9.5 |
| q_fanout | ok | 38642 | 7.37 | 3.88 | 3.14 | 0 | 1.38 | 2.82 | 7.3 |
| q_limits | ok | 38887 | 7.42 | 3.86 | 3.16 | 0 | 1.38 | 2.81 | 7.3 |
| q_payload4k | ok | 32604 | 127.36 | 4.60 | 3.42 | 0 | 1.79 | 3.12 | 7.2 |
| q_ordering | ok | 29590 | 5.64 | 5.07 | 3.88 | 0 | 1.59 | 2.85 | 40.1 |
| q_filter.froot | ok | 40920 | 3.90 | 3.67 | 3.37 | 0 | 2.16 | 2.30 | 7.9 |
| q_filter.fclient | ok | 38926 | 3.71 | 3.85 | 3.16 | 0 | 2.53 | 2.34 | 8.0 |
| q_mixed_plain.i0 | ok | 28994 | 5.53 | 5.17 | 2.87 | 0 | 1.57 | 3.25 | 7.1 |
| q_mixed_plain.i1 | ok | 28658 | 5.47 | 5.23 | 3.01 | 0 | 1.54 | 3.25 | 7.0 |
| q_mixed_plain.i2 | ok | 28440 | 5.42 | 5.27 | 3.02 | 0 | 1.64 | 3.38 | 6.8 |
| q_mixed_plain | ok | 85306 | 16.27 | 5.28 | 3.02 | 0 | 4.75 | 9.93 | 7.1 |
| q_mixed.i0 | ok | 23906 | 4.56 | 6.28 | 2.67 | 0 | 1.73 | 3.49 | 6.8 |
| q_mixed.i1 | ok | 23909 | 4.56 | 6.27 | 3.01 | 0 | 1.83 | 3.50 | 6.9 |
| q_mixed.i2 | ok | 17972 | 3.43 | 8.35 | 3.00 | 0 | 2.00 | 3.54 | 31.3 |
| q_mixed.i3 | ok | 26349 | 2.51 | 5.69 | 4.01 | 0 | 2.80 | 3.02 | 7.6 |
| q_mixed | ok | 71882 | 12.00 | 8.35 | 4.01 | 0 | 8.36 | 13.62 | 31.3 |

## Notes on 1.0.8

- The per-worker cap of 1.0.7 (about 915 msg/s) is gone: one worker now
  moves about 30000 msg/s, two about 38000, eight about 91000 on this
  laptop. Without ordering each package still makes two channel hops through
  the single queuer thread and one round trip per BLPOP and per RPUSH, which
  is what limits it now.
- The fast tier and the deep run agree within about 5 % except for ordering
  (39722 against 29590): a drain run ends only after the 1 s ordering buffer
  has released its last packages, while the time box counts what was taken
  from the source. The fast tier is enough to tell whether a change helps;
  the deep run is the reference.
- A second fast pass, started by mistake 20 minutes after the first, gave the
  same 1.0.7 numbers (within 1 %) and 1.0.8 numbers 5-12 % lower; the first
  pass, run on an idle laptop, is the one in the table.
- 1.0.8 uses about 40 MiB peak RSS with ordering at this rate (the 1 s
  ordering buffer holds about 40000 packages); 7-9 MiB otherwise.
- Stop time after SIGINT is still 2-4 s (fixed sleeps in the shutdown path).

# 1.1.0

Release 1.1.0: no queuer (each child delivers its own packages without
ordering; one sender delivers ordered packages for every child, pipelined),
packages as bytes, filters compiled once, per-target failure handling,
a logger that writes each line in one call. 1.0.8 and 1.1.0 were measured on the
same day, on the same machine, with the same harness, interleaved; the 1.0.7
and 1.0.8 tables above stay as they were.

## Environment

As for 1.0.7 above (same laptop, Redis 8.0.5 on 127.0.0.1:6390 without
persistence), rustc 1.99.1. Binaries: 1.0.8 from the tag v1.0.8 (b5f2b11) and
1.1.0 from the `release: 1.1.0` commit, both `cargo build --locked --release`;
for the last table also main before 1.1.0 (0a38bbf, the 1.1.0 code without the
last two stages). The program was pinned to cores 0-3 and the harness (with the
delay proxy and the feeder of `fast_ordering_remote`) to cores 4-11, the
machine otherwise idle. Date: 2026-11-17.

## How it was measured

The builds were run in turns, the first build of each round alternating, with a
check before every run that no other client was connected to the Redis:

```sh
# one run; <selection> as below, <build> 1.0.8 or 1.1.0
taskset -c 4-11 python3 bench/rmbench.py run <selection> --label rust-<build> \
    --cmd 'taskset -c 0-3 <binary> {config}' \
    --redis redis://127.0.0.1:6390 --results <dir>
# fast: --tag fast, 5 rounds
# deep: --tag quick --scale 30 --max-bytes 2147483648 --timeout 600, 3 rounds
# q_base.c8: --scenario q_base.c8 --scale 60 (same limits), 3 rounds
# full: --tag full, 3 rounds
# fast.c1 against main: --scenario fast.c1, 10 rounds, three builds
```

Every run of every build ended `ok` with 0 missing packages (58 runs). With
ordering the msg/s columns count the packages delivered to the targets in the
time box (the harness has counted that way since `fast_ordering_remote` was
added); the ordering figures of the 1.0.7 and 1.0.8 sections above counted the
packages popped from the source. CPU per package is the program's user plus
system time over the packages delivered. `fast_ordering_remote` is fed at 6000
packages per second to a target 1 ms away; 1.1.0 keeps up with the feed and
drains before the time box ends, so its figure is a lower bound.

## fast (median of 5, range in brackets)

| scenario | 1.0.8 msg/s | 1.1.0 msg/s | 1.1.0/1.0.8 | CPU us/pkg 1.0.8 | CPU us/pkg 1.1.0 | missing (worst) |
|---|---|---|---|---|---|---|
| fast.c1 | 32286 (30849-33533) | 44260 (40918-46701) | 1.37 | 17.1 | 11.9 | 0 |
| fast.c2 | 56121 (54749-58518) | 75170 (73113-75758) | 1.34 | 22.2 | 15.0 | 0 |
| fast.c8 | 68839 (67277-70160) | 85963 (80250-87211) | 1.25 | 24.6 | 14.4 | 0 |
| fast_fanout | 55984 (55193-57576) | 72374 (66456-75140) | 1.29 | 22.1 | 15.4 | 0 |
| fast_ordering | 45517 (43260-45732) | 85944 (81620-87870) | 1.89 | 22.1 | 10.9 | 0 |
| fast_filter | 57736 (56396-58888) | 93924 (90622-95477) | 1.63 | 45.8 | 23.0 | 0 |
| fast_limits | 55576 (54624-56266) | 71925 (71027-73059) | 1.29 | 22.2 | 15.4 | 0 |
| fast_ordering_remote | 1554 (971-1584) | 6004 (5992-6012) | 3.86 | 64.4 | 15.2 | 0 |

## deep: quick at scale 30 (median of 3, range in brackets)

| scenario | 1.0.8 msg/s | 1.1.0 msg/s | 1.1.0/1.0.8 | CPU us/pkg 1.0.8 | CPU us/pkg 1.1.0 | missing (worst) |
|---|---|---|---|---|---|---|
| q_base.c1 | 33700 (31891-33702) | 42594 (40825-43354) | 1.26 | 17.8 | 11.6 | 0 |
| q_base.c2 | 56387 (56016-58350) | 77549 (74817-78018) | 1.38 | 22.0 | 14.4 | 0 |
| q_base.c8 | 70793 (70068-72049) | 82943 (82454-83164) | 1.17 | 24.4 | 14.9 | 0 |
| q_fanout | 56926 (55945-57109) | 76478 (76310-77038) | 1.34 | 21.6 | 14.6 | 0 |
| q_limits | 57275 (57008-57381) | 76310 (73247-76722) | 1.33 | 21.6 | 14.5 | 0 |
| q_payload4k | 51092 (50630-52674) | 67509 (56607-70044) | 1.32 | 23.0 | 16.0 | 0 |
| q_ordering | 35457 (34710-36129) | 70420 (69422-71398) | 1.99 | 20.9 | 11.9 | 0 |
| q_filter.froot | 60865 (60460-61160) | 97209 (87904-98464) | 1.60 | 43.9 | 22.0 | 0 |
| q_filter.fclient | 55908 (55044-56860) | 97792 (95283-97984) | 1.75 | 48.5 | 21.8 | 0 |
| q_mixed_plain.i0 | 23246 (22943-23800) | 29956 (27902-30239) | 1.29 | 25.7 | 14.4 | 0 |
| q_mixed_plain.i1 | 23249 (22485-23802) | 29368 (29237-30496) | 1.26 | 26.1 | 14.5 | 0 |
| q_mixed_plain.i2 | 23052 (22910-23288) | 28005 (27380-29214) | 1.21 | 26.3 | 14.8 | 0 |
| q_mixed_plain | 69145 (67451-69739) | 84002 (82127-87624) | 1.21 | 26.0 | 14.6 | 0 |
| q_mixed.i0 | 18268 (18250-18325) | 23807 (23620-23953) | 1.30 | 27.5 | 15.6 | 0 |
| q_mixed.i1 | 18280 (18274-18292) | 23809 (23698-23955) | 1.30 | 27.9 | 15.7 | 0 |
| q_mixed.i2 | 14259 (14166-14266) | 22007 (21882-22068) | 1.54 | 28.8 | 14.2 | 0 |
| q_mixed.i3 | 20801 (20791-20860) | 23021 (22952-23156) | 1.11 | 62.9 | 24.7 | 0 |
| q_mixed | 57033 (56660-57059) | 88015 (87514-88260) | 1.54 | 33.0 | 16.5 | 0 |

## q_base.c8 at scale 60 (median of 3, range in brackets)

| scenario | 1.0.8 msg/s | 1.1.0 msg/s | 1.1.0/1.0.8 | CPU us/pkg 1.0.8 | CPU us/pkg 1.1.0 | missing (worst) |
|---|---|---|---|---|---|---|
| q_base.c8 | 72840 (72735-73337) | 83354 (81240-86897) | 1.14 | 23.4 | 15.0 | 0 |

## full (median of 3, range in brackets)

| scenario | 1.0.8 msg/s | 1.1.0 msg/s | 1.1.0/1.0.8 | CPU us/pkg 1.0.8 | CPU us/pkg 1.1.0 | missing (worst) |
|---|---|---|---|---|---|---|
| payload.p64.c1 | 34798 (30590-35881) | 47224 (46223-47320) | 1.36 | 21.0 | 10.8 | 0 |
| payload.p64.c2 | 59081 (59004-60837) | 77798 (67161-80898) | 1.32 | 22.0 | 13.6 | 0 |
| payload.p64.c4 | 74766 (74749-74944) | 92439 (84376-92624) | 1.24 | 24.6 | 13.0 | 0 |
| payload.p64.c8 | 74579 (74476-77510) | 87896 (84590-92095) | 1.18 | 24.5 | 13.4 | 0 |
| payload.p200.c1 | 36522 (35984-37258) | 46249 (45245-48556) | 1.27 | 20.3 | 11.1 | 0 |
| payload.p200.c2 | 57246 (55755-58995) | 78045 (72087-80809) | 1.36 | 22.4 | 13.9 | 0 |
| payload.p200.c4 | 72039 (72014-74693) | 88209 (88061-88440) | 1.22 | 24.6 | 13.6 | 0 |
| payload.p200.c8 | 77352 (74290-77368) | 87749 (83802-91843) | 1.13 | 24.5 | 13.8 | 0 |
| payload.p4096.c1 | 33020 (32918-34086) | 42253 (38098-43158) | 1.28 | 22.2 | 12.6 | 0 |
| payload.p4096.c2 | 53989 (53881-54107) | 69504 (69262-71784) | 1.29 | 24.2 | 14.4 | 0 |
| payload.p4096.c4 | 66814 (64686-66845) | 74608 (71934-77521) | 1.12 | 26.6 | 14.7 | 0 |
| payload.p4096.c8 | 68849 (68774-69019) | 74009 (73992-80236) | 1.07 | 27.1 | 14.8 | 0 |
| payload64k.c1 | 11396 (9746-12946) | 16133 (14975-16171) | 1.42 | 84.5 | 33.0 | 0 |
| payload64k.c2 | 17623 (16204-19441) | 21398 (17543-21472) | 1.21 | 92.0 | 41.0 | 0 |
| payload64k.c4 | 19328 (19323-21426) | 23999 (21286-24007) | 1.24 | 101.5 | 46.0 | 0 |
| payload64k.c8 | 21086 (19120-21162) | 21201 (21060-23852) | 1.01 | 110.0 | 51.5 | 0 |
| fanout.c1.mreplicant.n1 | 35368 (33618-38817) | 46228 (43246-48559) | 1.31 | 21.1 | 11.4 | 0 |
| fanout.c1.mreplicant.n3 | 18945 (18765-20250) | 23420 (22109-23983) | 1.24 | 10.1 | 7.7 | 0 |
| fanout.c1.mspreader.n1 | 31072 (30554-31563) | 45148 (42256-45151) | 1.45 | 20.3 | 10.9 | 0 |
| fanout.c1.mspreader.n3 | 30545 (30176-31488) | 43246 (42288-46220) | 1.42 | 20.8 | 11.2 | 0 |
| fanout.c4.mreplicant.n1 | 74734 (71979-74783) | 88204 (88106-92329) | 1.18 | 24.5 | 12.9 | 0 |
| fanout.c4.mreplicant.n3 | 39670 (39647-40696) | 46208 (44114-46231) | 1.16 | 12.8 | 8.6 | 0 |
| fanout.c4.mspreader.n1 | 72053 (67354-74956) | 86928 (81154-88009) | 1.21 | 24.8 | 13.2 | 0 |
| fanout.c4.mspreader.n3 | 72209 (69500-74994) | 87997 (84685-88123) | 1.22 | 24.0 | 13.3 | 0 |
| fanout.c8.mreplicant.n1 | 74526 (74406-77731) | 91769 (87618-91947) | 1.23 | 24.8 | 13.7 | 0 |
| fanout.c8.mreplicant.n3 | 42154 (41961-42275) | 45013 (43903-45958) | 1.07 | 13.3 | 8.8 | 0 |
| fanout.c8.mspreader.n1 | 77302 (74653-77701) | 87386 (84174-87714) | 1.13 | 24.4 | 13.3 | 0 |
| fanout.c8.mspreader.n3 | 77153 (74505-77411) | 91733 (87698-92054) | 1.19 | 24.3 | 13.8 | 0 |
| limits.n1 | 60917 (60615-61040) | 77833 (74573-80871) | 1.28 | 21.8 | 13.2 | 0 |
| limits.n3 | 33563 (32474-35294) | 41243 (35486-42183) | 1.23 | 12.0 | 9.0 | 0 |
| limits_time | 61060 (55806-62781) | 77745 (74922-80864) | 1.27 | 22.6 | 13.6 | 0 |
| ordering.c1 | 7840 (7791-7916) | 15761 (15630-15798) | 2.01 | 22.6 | 10.0 | 0 |
| ordering.c2 | 8281 (8263-8300) | 17434 (17289-17592) | 2.11 | 23.7 | 10.7 | 0 |
| ordering.c8 | 8280 (8260-8336) | 17715 (17556-17877) | 2.14 | 24.9 | 9.5 | 0 |
| filter.c1.froot | 34823 (32140-35475) | 55608 (50033-58733) | 1.60 | 42.2 | 17.8 | 0 |
| filter.c1.fclient | 34093 (31597-34186) | 53993 (52586-59176) | 1.58 | 43.7 | 18.0 | 0 |
| filter.c2.froot | 62824 (62456-64598) | 102218 (92660-102428) | 1.63 | 46.9 | 20.1 | 0 |
| filter.c2.fclient | 59099 (57088-60696) | 102158 (102140-102203) | 1.73 | 50.4 | 22.0 | 0 |
| filter.c8.froot | 88179 (87741-88396) | 113544 (113240-114091) | 1.29 | 52.6 | 22.1 | 0 |
| filter.c8.fclient | 88143 (87919-88196) | 113809 (107455-113838) | 1.29 | 58.8 | 22.2 | 0 |
| mixed_plain.i0 | 25765 (24509-26182) | 32631 (30821-32716) | 1.27 | 25.6 | 13.6 | 0 |
| mixed_plain.i1 | 25785 (24524-26199) | 31158 (29450-33224) | 1.21 | 29.4 | 13.5 | 0 |
| mixed_plain.i2 | 25799 (24538-26215) | 30205 (28185-30615) | 1.17 | 34.2 | 14.0 | 0 |
| mixed_plain | 77294 (73527-78546) | 90492 (84442-91702) | 1.17 | 29.5 | 13.7 | 0 |
| mixed.i0 | 21962 (21663-22159) | 29091 (28710-29644) | 1.32 | 28.0 | 13.8 | 0 |
| mixed.i1 | 21726 (21433-22171) | 29555 (28732-29669) | 1.36 | 31.7 | 13.8 | 0 |
| mixed.i2 | 7017 (6821-7451) | 11947 (11890-12042) | 1.70 | 34.2 | 12.8 | 0 |
| mixed.i3 | 23898 (23835-24418) | 27114 (26786-27207) | 1.13 | 85.6 | 22.3 | 0 |
| mixed | 28060 (27275-29793) | 47762 (47533-48137) | 1.70 | 39.2 | 14.7 | 0 |

## fast.c1 against main (median of 10, range in brackets)

| scenario | 1.0.8 msg/s | main 0a38bbf msg/s | 1.1.0 msg/s | 1.1.0/1.0.8 | 1.1.0/main 0a38bbf | CPU us/pkg 1.0.8 | CPU us/pkg main 0a38bbf | CPU us/pkg 1.1.0 | missing (worst) |
|---|---|---|---|---|---|---|---|---|---|
| fast.c1 | 30674 (29937-31527) | 44681 (41599-47972) | 44698 (42874-47154) | 1.46 | 1.00 | 17.3 | 11.8 | 11.9 | 0 |

## Notes on 1.1.0

- No scenario is slower than 1.0.8. The smallest gain is payload64k.c8
  (packages of 64 KiB with 8 children), where both builds are bound by Redis on
  the same machine.
- With ordering (fast_ordering, q_ordering, ordering.*) 1.1.0 delivers about
  twice what 1.0.8 does, in strict order for any number of children; towards a
  target 1 ms away (fast_ordering_remote) it follows the 6000/s feed, where 1.0.8
  delivers about 1550/s.
- fast.c1 was also measured against main before 1.1.0 over 10 rounds because one
  pre-release run of 5 rounds had read 0.92; over 10 rounds 1.1.0 and main are
  equal (1.00), with the same instructions per package counted by callgrind.
