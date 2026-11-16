use crate::constants::PROGRAM_NAME;
use std::cell::RefCell;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;

#[allow(dead_code)]
pub static COLOR_NOHEAD: u8 = 1;
#[allow(dead_code)]
pub static COLOR_NOHEADER: u8 = 1;
#[allow(dead_code)]
pub static COLOR_NOTAIL: u8 = 2;
#[allow(dead_code)]
pub static COLOR_NOHEAD_NOTAIL: u8 = 3;
#[allow(dead_code)]
pub static COLOR_NOHEADER_NOTAIL: u8 = 3;

// Set colors available for everybody
#[allow(dead_code)]
pub static COLOR_CLOSE: &[u8; 6] = b"\x1B[1;0m";
#[allow(dead_code)]
pub static COLOR_SIMPLE_GREY: &[u8; 7] = b"\x1B[0;30m"; // 0, 30"
#[allow(dead_code)]
pub static COLOR_SIMPLE_RED: &[u8; 7] = b"\x1B[0;31m"; // 0, 31"
#[allow(dead_code)]
pub static COLOR_SIMPLE_GREEN: &[u8; 7] = b"\x1B[0;32m"; // 0, 32"
#[allow(dead_code)]
pub static COLOR_SIMPLE_YELLOW: &[u8; 7] = b"\x1B[0;33m"; // 0, 33"
#[allow(dead_code)]
pub static COLOR_SIMPLE_BLUE: &[u8; 7] = b"\x1B[0;34m"; // 0, 34"
#[allow(dead_code)]
pub static COLOR_SIMPLE_PURPLE: &[u8; 7] = b"\x1B[0;35m"; // 0, 35"
#[allow(dead_code)]
pub static COLOR_SIMPLE_CYAN: &[u8; 7] = b"\x1B[0;36m"; // 0, 36"
#[allow(dead_code)]
pub static COLOR_SIMPLE_WHITE: &[u8; 7] = b"\x1B[0;37m"; // 0, 37"
#[allow(dead_code)]
pub static COLOR_GREY: &[u8; 7] = b"\x1B[1;30m"; // 1, 30"
#[allow(dead_code)]
pub static COLOR_RED: &[u8; 7] = b"\x1B[1;31m"; // 1, 31"
#[allow(dead_code)]
pub static COLOR_GREEN: &[u8; 7] = b"\x1B[1;32m"; // 1, 32"
#[allow(dead_code)]
pub static COLOR_YELLOW: &[u8; 7] = b"\x1B[1;33m"; // 1, 33"
#[allow(dead_code)]
pub static COLOR_BLUE: &[u8; 7] = b"\x1B[1;34m"; // 1, 34"
#[allow(dead_code)]
pub static COLOR_PURPLE: &[u8; 7] = b"\x1B[1;35m"; // 1, 35"
#[allow(dead_code)]
pub static COLOR_CYAN: &[u8; 7] = b"\x1B[1;36m"; // 1, 36"
#[allow(dead_code)]
pub static COLOR_WHITE: &[u8; 7] = b"\x1B[1;37m"; // 1, 37"

// Every line the program writes goes through `log` (M3-j = ja, A-F5d): built whole in a
// buffer of the thread and written with one call, so lines of two threads never mix and
// stderr costs one system call per line. A write that fails (a closed pipe) is dropped,
// never a panic (M1F-9, M3R-36; D5-1 = a): the program goes on delivering.

/// Where a line goes
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stream {
    Out,
    Err,
}

/// Standard output, for `print_debug!`
pub fn stdout() -> Stream {
    Stream::Out
}

/// Standard error, for `print_debug!`
pub fn stderr() -> Stream {
    Stream::Err
}

// Whether each stream was a terminal when first written to: colours only there (M3-j = ja)
static TERMINAL: OnceLock<[bool; 2]> = OnceLock::new();

thread_local! {
    // The line being built, kept for the thread's next line (no allocation per line)
    static LINE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Colours on for this stream (a terminal)
#[allow(clippy::disallowed_methods)]
pub fn colours(stream: Stream) -> bool {
    TERMINAL.get_or_init(|| [io::stdout().is_terminal(), io::stderr().is_terminal()])
        [stream as usize]
}

/// The head of a line: the date and the program's name
pub fn head(line: &mut Vec<u8>) {
    let _ = write!(
        line,
        "{} {}: ",
        chrono::offset::Local::now().format("%Y-%m-%d %H:%M:%S"),
        PROGRAM_NAME
    );
}

/// One part of a line in its colour: the colour, the text, the closing code (no codes
/// when the stream is not a terminal)
pub fn part(line: &mut Vec<u8>, colours: bool, colour: &[u8], text: fmt::Arguments) {
    if colours {
        line.extend_from_slice(colour);
    }
    let _ = line.write_fmt(text);
    if colours {
        line.extend_from_slice(COLOR_CLOSE);
    }
}

/// Builds a line with `build` (given the buffer and whether colours are on), ends it and
/// writes it in one call; a failed write is dropped
pub fn emit(stream: Stream, build: impl FnOnce(&mut Vec<u8>, bool)) {
    LINE.with(|cell| {
        // A line logged while another is built on this thread (cannot happen today) gets
        // a buffer of its own instead of a panic
        let mut own = Vec::new();
        let mut borrowed = cell.try_borrow_mut();
        let line = match borrowed.as_mut() {
            Ok(b) => &mut **b,
            Err(_) => &mut own,
        };
        line.clear();
        build(line, colours(stream));
        line.push(b'\n');
        write_line(stream, line);
    });
}

/// The one place that writes to the process's streams (R4 of gate 6: clippy.toml
/// disallows `std::io::stdout` and `std::io::stderr` elsewhere)
#[allow(clippy::disallowed_methods)]
fn write_line(stream: Stream, line: &[u8]) {
    let _ = match stream {
        Stream::Out => io::stdout().lock().write_all(line),
        Stream::Err => io::stderr().lock().write_all(line),
    };
}

/// Standard output's buffer flushed before the process exits (its lines end with a newline,
/// so nothing should be left; a failure is dropped)
#[allow(clippy::disallowed_methods)]
pub fn flush() {
    let _ = io::stdout().lock().flush();
}

/// A line of one colour: with the head unless `format` says no head (`COLOR_NOHEAD`)
pub fn log(stream: Stream, colour: &[u8], format: u8, text: fmt::Arguments) {
    emit(stream, |line, on| {
        one_colour(line, on, colour, format, text)
    });
}

/// The text of a one-colour line (without its newline): the colour, the head, the text,
/// the closing code; no codes when `on` is false. Today's text, with the closing code
/// before the newline instead of after it
pub fn one_colour(line: &mut Vec<u8>, on: bool, colour: &[u8], format: u8, text: fmt::Arguments) {
    if on {
        line.extend_from_slice(colour);
    }
    if format & COLOR_NOHEAD == 0 {
        head(line);
    }
    let _ = line.write_fmt(text);
    if on {
        line.extend_from_slice(COLOR_CLOSE);
    }
}

/// A line without colour or head (the empty line after `--help`, the version)
pub fn plain(stream: Stream, text: fmt::Arguments) {
    emit(stream, |line, _| {
        let _ = line.write_fmt(text);
    });
}

macro_rules! print_debug {
    ($program_name: expr, $output: expr, $color: expr, $format: expr, $msg: expr) => {
        $crate::debugger::log($output, $color, $format, format_args!("{}", $msg))
    };
    ($program_name: expr, $output: expr, $color: expr, $format: expr, $msg_format: expr, $($args:expr),+) => {
        $crate::debugger::log($output, $color, $format, format_args!($msg_format $(, $args)+))
    };
}
pub(crate) use print_debug;

macro_rules! option2string {
    ($value: expr) => {
        match $value {
            None => "-".to_string(),
            Some(v) => format!("{}", v),
        }
    };
}
pub(crate) use option2string;
