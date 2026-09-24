//! [`ContextWords`]: split a structural name into the context words a
//! recognizer's context boost matches on.

/// Turn a structural *name* — an XML element or attribute name, a JSON object
/// key — into context words for the value it labels, so a recognizer's context
/// boost can fire on it (an `<ssn>` element or an `"ssn"` key vouches for its
/// content the way a CSV header vouches for its cell).
pub trait ContextWords {
    /// Split into component words, borrowed from the name: a `camelCase` /
    /// `PascalCase` name breaks on each lower→upper transition, and `_` / `-`
    /// are separators, so `paymentCard`, `PaymentCard`, `payment_card`, and
    /// `payment-card` all split into `["payment", "card"]` (case preserved on
    /// each word). A name with no case transition or separator (`ssn`, `email`)
    /// yields a single word; consecutive capitals (`XMLParser`) do not split.
    ///
    /// Each word is a slice of the input — no per-word allocation.
    fn split_context_words(&self) -> impl Iterator<Item = &str>;

    /// The [`split_context_words`](Self::split_context_words) joined into one
    /// space-separated string, where a context keyword like `card` matches on a
    /// word boundary.
    #[must_use]
    fn context_words(&self) -> String {
        let mut out = String::new();
        for word in self.split_context_words() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
        out
    }
}

impl ContextWords for str {
    fn split_context_words(&self) -> impl Iterator<Item = &str> {
        ContextWordsIter {
            name: self,
            chars: self.char_indices().peekable(),
            start: None,
        }
    }
}

/// Walks a name's characters, yielding each word as a slice. A word is a
/// maximal run of non-separator characters with no lower→upper boundary inside
/// it; separators (`_`, `-`) are consumed and never part of a word.
struct ContextWordsIter<'a> {
    name: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    /// Byte offset the current word started at, once one is open.
    start: Option<usize>,
}

impl<'a> Iterator for ContextWordsIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let mut prev: Option<char> = None;
        while let Some(&(idx, c)) = self.chars.peek() {
            if c == '_' || c == '-' {
                // A separator closes any open word and is itself dropped.
                if let Some(start) = self.start.take() {
                    self.chars.next();
                    return Some(&self.name[start..idx]);
                }
                self.chars.next();
                prev = None;
                continue;
            }
            // A lower→upper (or digit→upper) transition closes the open word
            // *before* this character, which begins the next one.
            if c.is_uppercase()
                && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                && let Some(start) = self.start.take()
            {
                return Some(&self.name[start..idx]);
            }
            if self.start.is_none() {
                self.start = Some(idx);
            }
            self.chars.next();
            prev = Some(c);
        }
        // Input exhausted: emit the trailing open word, if any.
        self.start.take().map(|start| &self.name[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(name: &str) -> Vec<&str> {
        name.split_context_words().collect()
    }

    #[test]
    fn splits_case_and_separators() {
        assert_eq!(split("paymentCard"), ["payment", "Card"]);
        assert_eq!(split("PaymentCard"), ["Payment", "Card"]);
        assert_eq!(split("payment_card"), ["payment", "card"]);
        assert_eq!(split("payment-card"), ["payment", "card"]);
        assert_eq!(split("ssn"), ["ssn"]);
        assert_eq!(split("taxId"), ["tax", "Id"]);
        assert_eq!(split("XMLParser"), ["XMLParser"]); // consecutive caps do not split
        assert_eq!(split("host"), ["host"]);
    }

    #[test]
    fn handles_repeated_and_edge_separators() {
        assert_eq!(split("__a--b__"), ["a", "b"]);
        assert_eq!(split(""), Vec::<&str>::new());
        assert_eq!(split("_"), Vec::<&str>::new());
    }

    #[test]
    fn context_words_joins_with_spaces() {
        assert_eq!("paymentCard".context_words(), "payment Card");
        assert_eq!("payment_card".context_words(), "payment card");
        assert_eq!("ssn".context_words(), "ssn");
    }
}
