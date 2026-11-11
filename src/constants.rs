use std::time::Duration;

pub static PROGRAM_NAME: &str = "RedisMultiplexer";
pub static STATISTICS_SECONDS: u128 = 10;
pub static DEFAULT_CHECK_SECONDS: u64 = 1;
pub static MAX_QUEUE_SIZE: isize = 100000000;

// Targets (plan 2.2, M3-g = ga, brought forward by M3R-12 X1b)
pub const TARGET_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const TARGET_IO_TIMEOUT: Duration = Duration::from_secs(5);
pub const TARGET_BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const TARGET_BACKOFF_MAX: Duration = Duration::from_secs(5);

// Autofields
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
