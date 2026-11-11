//! Stop under load with more than one child (M3R-3).
//!
//! The one-child cases are in lifecycle.rs (ordering off) and ordering.rs (ordering on).

mod common;

use common::*;
use std::time::Duration;

#[test]
fn stop_under_load_two_children() {
    stop_under_load("stop_load_c2", 2, false, Duration::from_millis(500));
}

#[test]
fn stop_under_load_two_children_with_ordering() {
    // After the first release, so packages are both delivered and held when the stop comes
    stop_under_load("stop_load_c2_ordering", 2, true, Duration::from_millis(300));
}

#[test]
fn stop_under_load_four_children() {
    stop_under_load("stop_load_c4", 4, false, Duration::from_millis(500));
}

#[test]
fn stop_under_load_four_children_with_ordering() {
    stop_under_load("stop_load_c4_ordering", 4, true, Duration::from_millis(300));
}
