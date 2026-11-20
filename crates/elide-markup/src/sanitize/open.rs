//! [`OpenElements`]: a stack of open element names that answers whether a
//! name is open without scanning it.

use std::collections::HashMap;

/// The elements open, innermost last, each with `T` alongside its name, and
/// how many of each name are open, so an end tag naming none of them is
/// turned away at once and a matching one pops only what it closes.
pub(super) struct OpenElements<T = ()> {
    stack: Vec<(String, T)>,
    counts: HashMap<String, usize>,
}

impl<T> OpenElements<T> {
    pub(super) fn new() -> Self {
        Self {
            stack: Vec::new(),
            counts: HashMap::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// The innermost open element.
    pub(super) fn last(&self) -> Option<&(String, T)> {
        self.stack.last()
    }

    pub(super) fn push(&mut self, name: String, value: T) {
        *self.counts.entry(name.clone()).or_default() += 1;
        self.stack.push((name, value));
    }

    /// Close the innermost open element.
    pub(super) fn pop(&mut self) -> Option<(String, T)> {
        let (name, value) = self.stack.pop()?;
        if let Some(count) = self.counts.get_mut(&name) {
            *count -= 1;
            if *count == 0 {
                self.counts.remove(&name);
            }
        }
        Some((name, value))
    }

    /// Close the innermost open element named `name` and every one inside
    /// it; whether one was open.
    pub(super) fn close(&mut self, name: &str) -> bool {
        if !self.counts.contains_key(name) {
            return false;
        }
        while let Some((open, _)) = self.pop() {
            if open == name {
                break;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_pops_through_the_innermost_match() {
        let mut open = OpenElements::new();
        for name in ["div", "span", "div", "b"] {
            open.push(name.to_owned(), ());
        }
        assert!(!open.close("p"));
        assert!(open.close("div"));
        assert_eq!(open.last().map(|(name, _)| name.as_str()), Some("span"));
        assert!(open.close("div"));
        assert!(open.is_empty());
        assert!(!open.close("span"));
    }
}
