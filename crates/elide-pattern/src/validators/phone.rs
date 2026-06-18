//! Region-aware phone-number validator backed by the
//! `phonenumber` crate (Rust port of Google's libphonenumber).
//!
//! Two paths:
//!
//! 1. Inputs that parse as E.164 (carry their own `+CC` prefix)
//!    validate directly, regardless of caller context.
//! 2. Inputs in national format (no leading `+`) need a region
//!    hint. When [`ValidationContext::country`] is set we use it;
//!    otherwise we fail closed — region-less national-format
//!    matching is genuinely ambiguous (a 13-digit run can be a
//!    valid IL/IN phone *and* the leading 13 digits of a Visa
//!    PAN), so without a country signal we'd rather miss a
//!    handful of national-format numbers than mislabel card and
//!    account numbers as phones.

use std::str::FromStr;

use phonenumber::country::Id;
use phonenumber::parse;

use super::ValidationContext;

/// Return `true` when `value` parses as a valid phone number
/// for the caller's jurisdiction (or as E.164 with an explicit
/// `+CC` prefix).
pub fn phone(value: &str, ctx: &ValidationContext) -> bool {
    let trimmed = value.trim();

    if parse(None, trimmed).map(|n| n.is_valid()).unwrap_or(false) {
        return true;
    }

    // Try each asserted jurisdiction; a national-format number need only
    // be valid for one of them.
    ctx.countries.iter().any(|c| {
        Id::from_str(c.as_str())
            .ok()
            .and_then(|region| parse(Some(region), trimmed).ok())
            .map(|n| n.is_valid())
            .unwrap_or(false)
    })
}
