//! Age generator. Date-of-birth and date-time pattern-preserve
//! their original and don't go through this module.

use fake::Fake;
use fake::rand::RngExt;

pub(super) fn age<R: RngExt + ?Sized>(rng: &mut R) -> String {
    let years: u8 = (1..=99u8).fake_with_rng(rng);
    years.to_string()
}
