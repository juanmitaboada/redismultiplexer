pub static PROGRAM_NAME: &str = "RedisMultiplexer";
pub static STATISTICS_SECONDS: u128 = 10;
pub static DEFAULT_CHECK_SECONDS: u64 = 1;
pub static MAX_QUEUE_SIZE: isize = 100000000;

// Autofields
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
