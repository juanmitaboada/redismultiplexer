# Bench baseline

Reference numbers for redismultiplexer 1.0.7, measured with
`bench/rmbench.py` before any performance work. Later versions are compared
against them with `rmbench.py compare`.

Status: pending, to be filled from runs on the maintainer's laptop.

## Environment

| Item | Value |
|---|---|
| CPU / cores | pending |
| OS | pending |
| Redis | pending (local, loopback, no persistence) |
| Binary | 1.0.7 program code, `cargo build --locked --release`, rustc pending |
| Harness | pending |
| Date | pending |

## How it was measured

```sh
python3 bench/rmbench.py run --tag quick --repeat 3 --label rust-1.0.7 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/1.0.7
python3 bench/rmbench.py run --tag full --label rust-1.0.7 \
    --cmd 'target/release/redismultiplexer {config}' \
    --redis redis://127.0.0.1:6390 --results ~/rmbench/1.0.7
```

## quick (median of 3)

pending

## full

pending

## Notes

pending
