use std::time::Duration;

pub static PROGRAM_NAME: &str = "RedisMultiplexer";
pub static STATISTICS_SECONDS: u128 = 10;
// Defaults of a target with a hardlimit and no timelimit or checklimit (M3-f, L4c-5)
pub const DEFAULT_TIMELIMIT: u64 = 1;
pub const DEFAULT_CHECKLIMIT: u64 = 10000;
// How often a target without timelimit and checklimit is checked (L1a: monotonic clock)
pub const DEFAULT_CHECK: Duration = Duration::from_secs(1);

// Connections (plan 2.2, M3-g = ga): defaults of the `connect_timeout` and
// `command_timeout` keys, root (source) and per client (M3R-19)
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
// The smallest `connect_timeout` or `command_timeout` other than 0 (M3R-27)
pub const MIN_TIMEOUT: Duration = Duration::from_millis(1);
// How long a delivering thread waits for a connect attempt before it goes on without the
// target (X1c): a reachable target connects within it and takes the package
pub const CONNECT_GRACE: Duration = Duration::from_millis(100);
// How often a thread waiting at its start for its targets' first connects looks at the
// stop (M3R-23)
pub const START_POLL: Duration = Duration::from_millis(100);
// BLPOP's own wait: the server answers nil after it when the source is empty
pub const BLPOP_WAIT: Duration = Duration::from_secs(1);
pub const TARGET_BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const TARGET_BACKOFF_MAX: Duration = Duration::from_secs(5);
// The sender delivers a released batch in chunks (plan 9.2, P3 = p3a): at most this many
// packages and this many bytes of them per chunk; a larger package goes alone
pub const CHUNK_PACKAGES: usize = 1000;
pub const CHUNK_BYTES: usize = 4 << 20;
// A warning that can come once per package is printed at most once per this time, per
// thread and kind, with the count of the others (M3-j = ja, plan 2.8)
pub const REPEAT_WINDOW: Duration = Duration::from_secs(10);

// Autofields
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
