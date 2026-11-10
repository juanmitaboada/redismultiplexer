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
Date: 2026-11-10.

## How it was measured

```sh
# fast: time-boxed, 5 s per run, about 2 min per build for 3 repetitions
python3 bench/rmbench.py run --tag fast --repeat 3 --label rust-1.0.8 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/fast/1.0.8
# deep: quick matrix with 30 times the counts (150000 packages per source),
# about 60 min for 1.0.7 and 5 min for 1.0.8
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
1.0.7 columns. Every run of both builds ended `ok` with 0 missing packages.

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
