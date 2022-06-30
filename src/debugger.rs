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
pub static COLOR_SIMPLE_GREY: &[u8; 7] =     b"\x1B[0;30m"; // 0, 30"
#[allow(dead_code)]
pub static COLOR_SIMPLE_RED: &[u8; 7] =      b"\x1B[0;31m"; // 0, 31"
#[allow(dead_code)]
pub static COLOR_SIMPLE_GREEN: &[u8; 7] =    b"\x1B[0;32m"; // 0, 32"
#[allow(dead_code)]
pub static COLOR_SIMPLE_YELLOW: &[u8; 7] =   b"\x1B[0;33m"; // 0, 33"
#[allow(dead_code)]
pub static COLOR_SIMPLE_BLUE: &[u8; 7] =     b"\x1B[0;34m"; // 0, 34"
#[allow(dead_code)]
pub static COLOR_SIMPLE_PURPLE: &[u8; 7] =   b"\x1B[0;35m"; // 0, 35"
#[allow(dead_code)]
pub static COLOR_SIMPLE_CYAN: &[u8; 7] =     b"\x1B[0;36m"; // 0, 36"
#[allow(dead_code)]
pub static COLOR_SIMPLE_WHITE: &[u8; 7] =    b"\x1B[0;37m"; // 0, 37"
#[allow(dead_code)]
pub static COLOR_GREY: &[u8; 7] =           b"\x1B[1;30m"; // 1, 30"
#[allow(dead_code)]
pub static COLOR_RED: &[u8; 7] =            b"\x1B[1;31m"; // 1, 31"
#[allow(dead_code)]
pub static COLOR_GREEN: &[u8; 7] =          b"\x1B[1;32m"; // 1, 32"
#[allow(dead_code)]
pub static COLOR_YELLOW: &[u8; 7] =         b"\x1B[1;33m"; // 1, 33"
#[allow(dead_code)]
pub static COLOR_BLUE: &[u8; 7] =           b"\x1B[1;34m"; // 1, 34"
#[allow(dead_code)]
pub static COLOR_PURPLE: &[u8; 7] =         b"\x1B[1;35m"; // 1, 35"
#[allow(dead_code)]
pub static COLOR_CYAN: &[u8; 7] =           b"\x1B[1;36m"; // 1, 36"
#[allow(dead_code)]
pub static COLOR_WHITE: &[u8; 7] =          b"\x1B[1;37m"; // 1, 37"


macro_rules! print_debug {
    ($program_name: expr, $output: expr, $color: expr, $format: expr, $msg: expr) => {{
        // Set color
        $output.write($color).unwrap();

        // Head
        if     ($format != COLOR_NOHEAD)
            && ($format != COLOR_NOHEADER)
            && ($format != COLOR_NOHEAD_NOTAIL)
            && ($format != COLOR_NOHEADER_NOTAIL)
        {
            let ts = chrono::offset::Local::now().format("%Y-%m-%d %H:%M:%S");
            let header: String = format!("{} {}: ", ts, PROGRAM_NAME);
            $output.write(header.as_bytes()).unwrap();
        }

        // Show message
        $output.write($msg.as_bytes()).unwrap();

        // Tail
        if     ($format != COLOR_NOTAIL)
            && ($format != COLOR_NOHEAD_NOTAIL)
            && ($format != COLOR_NOHEADER_NOTAIL)
        {
            $output.write(b"\n").unwrap();
        }

        // Set closing color
        $output.write(COLOR_CLOSE).unwrap();

    }};
    ($program_name: expr, $output: expr, $color: expr, $format: expr, $msg_format: expr, $($args:expr),+) => {{
        // Set color
        $output.write($color).unwrap();

        // Head
        if     ($format != COLOR_NOHEAD)
            && ($format != COLOR_NOHEADER)
            && ($format != COLOR_NOHEAD_NOTAIL)
            && ($format != COLOR_NOHEADER_NOTAIL)
        {
            let ts = chrono::offset::Local::now().format("%Y-%m-%d %H:%M:%S");
            let header: String = format!("{} {}: ", ts, PROGRAM_NAME);
            $output.write(header.as_bytes()).unwrap();
        }

        let message: String = format!($msg_format$(, $args)+);
        $output.write(message.as_bytes()).unwrap();

        // Tail
        if     ($format != COLOR_NOTAIL)
            && ($format != COLOR_NOHEAD_NOTAIL)
            && ($format != COLOR_NOHEADER_NOTAIL)
        {
            $output.write(b"\n").unwrap();
        }

        // Set closing color
        $output.write(COLOR_CLOSE).unwrap();

    }};

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
