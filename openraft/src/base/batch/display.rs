//! Display implementation for `Batch`.

use std::fmt;
use std::fmt::Formatter;

use super::Batch;

impl<T: fmt::Display> Batch<T> {
    /// Returns a display helper that shows all elements.
    pub fn display(&self) -> impl fmt::Display + '_ {
        BatchDisplay {
            elements: self,
            max: None,
        }
    }

    /// Returns a display helper that shows at most `max` elements.
    pub fn display_n(&self, max: usize) -> impl fmt::Display + '_ {
        BatchDisplay {
            elements: self,
            max: Some(max),
        }
    }
}

struct BatchDisplay<'a, T> {
    elements: &'a Batch<T>,
    max: Option<usize>,
}

impl<'a, T: fmt::Display> fmt::Display for BatchDisplay<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slice = self.elements.as_slice();
        let max = self.max.unwrap_or(slice.len());
        let len = slice.len();
        let shown = max.min(len);

        write!(f, "[")?;
        for (i, e) in slice.iter().take(max).enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", e)?;
        }
        if len > max {
            if shown > 0 {
                write!(f, ", ")?;
            }
            write!(f, "... {} more", len - max)?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Batch::Single(42).display()), "[42]");
        assert_eq!(format!("{}", Batch::Vec(vec![1, 2]).display()), "[1, 2]");
        assert_eq!(format!("{}", Batch::<i32>::Vec(vec![]).display()), "[]");
    }

    #[test]
    fn test_display_n() {
        let v: Batch<i32> = [1, 2, 3, 4, 5].into();

        assert_eq!(format!("{}", v.display_n(3)), "[1, 2, 3, ... 2 more]");
        assert_eq!(format!("{}", v.display_n(5)), "[1, 2, 3, 4, 5]");
        assert_eq!(format!("{}", v.display_n(10)), "[1, 2, 3, 4, 5]");
        assert_eq!(format!("{}", v.display_n(0)), "[... 5 more]");

        let v2: Batch<i32> = 42.into();
        assert_eq!(format!("{}", v2.display_n(0)), "[... 1 more]");
        assert_eq!(format!("{}", v2.display_n(1)), "[42]");
    }
}
