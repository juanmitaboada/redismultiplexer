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
