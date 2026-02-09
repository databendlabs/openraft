//! Display implementation for `Batch`.

use std::fmt;
use std::fmt::Formatter;

use super::RaftBatch;
use crate::OptionalSend;

/// Display helper for types implementing `RaftBatch`.
pub struct DisplayBatch<'a, T, B>
where
    T: fmt::Display + OptionalSend + 'static + fmt::Debug,
    B: RaftBatch<T>,
{
    pub(super) elements: &'a B,
    pub(super) max: Option<usize>,
    pub(super) _phantom: std::marker::PhantomData<T>,
}

impl<'a, T, B> DisplayBatch<'a, T, B>
where
    T: fmt::Display + OptionalSend + 'static + fmt::Debug,
    B: RaftBatch<T>,
{
    pub(super) fn new(elements: &'a B, max: Option<usize>) -> Self {
        Self {
            elements,
            max,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, T, B> fmt::Display for DisplayBatch<'a, T, B>
where
    T: fmt::Display + OptionalSend + 'static + fmt::Debug,
    B: RaftBatch<T>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let len = self.elements.len();
        let max = self.max.unwrap_or(len);
        let shown = max.min(len);

        write!(f, "[")?;
        for (i, e) in self.elements.iter().take(max).enumerate() {
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
    use crate::impls::Batch;

    #[test]
    fn test_display() {
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display(&Batch::Single(42))),
            "[42]"
        );
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display(&Batch::Vec(vec![1, 2]))),
            "[1, 2]"
        );
        assert_eq!(
            format!(
                "{}",
                <Batch<i32> as RaftBatch<i32>>::display(&Batch::<i32>::Vec(vec![]))
            ),
            "[]"
        );
    }

    #[test]
    fn test_display_n() {
        let v: Batch<i32> = [1, 2, 3, 4, 5].into();

        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v, 3)),
            "[1, 2, 3, ... 2 more]"
        );
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v, 5)),
            "[1, 2, 3, 4, 5]"
        );
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v, 10)),
            "[1, 2, 3, 4, 5]"
        );
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v, 0)),
            "[... 5 more]"
        );

        let v2: Batch<i32> = 42.into();
        assert_eq!(
            format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v2, 0)),
            "[... 1 more]"
        );
        assert_eq!(format!("{}", <Batch<i32> as RaftBatch<i32>>::display_n(&v2, 1)), "[42]");
    }
}
