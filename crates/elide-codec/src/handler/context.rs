//! Shared helper for turning a structural *name*, an XML element or
//! attribute name, a JSON object key, into context words for the value it
//! labels, so a recognizer's context boost can fire on it (an `<ssn>` element
//! or an `"ssn"` key vouches for its content the way a CSV header vouches for
//! its cell).

/// Split a `name` into space-separated words: a `camelCase` / `PascalCase`
/// name breaks on each lower→upper transition, and `_` / `-` are separators,
/// so `paymentCard`, `PaymentCard`, `payment_card`, and `payment-card` all
/// become `"payment card"`, where a context keyword like `card` then matches
/// on a word boundary. A name with no case transition or separator (`ssn`,
/// `email`) is returned unchanged.
#[must_use]
pub(crate) fn context_words(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c == '_' || c == '-' {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            if c.is_uppercase()
                && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                && !out.ends_with(' ')
            {
                out.push(' ');
            }
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_case_and_separators() {
        for (name, expected) in [
            ("paymentCard", "payment Card"),
            ("PaymentCard", "Payment Card"),
            ("payment_card", "payment card"),
            ("payment-card", "payment card"),
            ("ssn", "ssn"),
            ("taxId", "tax Id"),
            ("XMLParser", "XMLParser"), // consecutive caps do not split
            ("host", "host"),
        ] {
            assert_eq!(context_words(name), expected, "name = {name:?}");
        }
    }
}
