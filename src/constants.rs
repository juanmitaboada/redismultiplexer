use std::time::Duration;

pub static PROGRAM_NAME: &str = "RedisMultiplexer";
pub static STATISTICS_SECONDS: u128 = 10;
pub static DEFAULT_CHECK_SECONDS: u64 = 1;
pub static MAX_QUEUE_SIZE: isize = 100000000;

// Connections (plan 2.2, M3-g = ga): defaults of the `connect_timeout` and
// `command_timeout` keys, root (source) and per client (M3R-19)
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
// BLPOP's own wait: the server answers nil after it when the source is empty
pub const BLPOP_WAIT: Duration = Duration::from_secs(1);
pub const TARGET_BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const TARGET_BACKOFF_MAX: Duration = Duration::from_secs(5);

// Autofields
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
