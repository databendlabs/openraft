//! Implement [`std::fmt::Display`] for types such as `Option<T>` and slice `&[T]`.

use std::fmt;

/// Implement `Display` for `Option<T>` if T is `Display`.
///
/// It outputs a literal string `"None"` if it is None. Otherwise it invokes the Display
/// implementation for T.
pub(crate) struct DisplayOption<'a, T: fmt::Display>(pub &'a Option<T>);

impl<'a, T: fmt::Display> fmt::Display for DisplayOption<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => {
                write!(f, "None")
            }
            Some(x) => x.fmt(f),
        }
    }
}

/// Implement `Display` for `&[T]` if T is `Display`.
///
/// It outputs at most 5 elements, excluding those from the 5th to the second-to-last one:
/// - `DisplaySlice(&[1,2,3,4,5,6])` outputs: `"[1,2,3,4,...,6]"`.
pub(crate) struct DisplaySlice<'a, T: fmt::Display>(pub &'a [T]);

impl<'a, T: fmt::Display> fmt::Display for DisplaySlice<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        let slice = self.0;
        let max = 5;
        let len = slice.len();

        write!(f, "[")?;

        if len > max {
            for (i, t) in slice[..(max - 1)].iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }

            write!(f, ",..,")?;
            write!(f, "{}", slice.last().unwrap())?;
        } else {
            for (i, t) in slice.iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }
        }

        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use crate::display_ext::DisplaySlice;

    #[test]
    fn test_display_slice() {
        let a = vec![1, 2, 3, 4];
        assert_eq!("[1,2,3,4]", DisplaySlice(&a).to_string());

        let a = vec![1, 2, 3, 4, 5];
        assert_eq!("[1,2,3,4,5]", DisplaySlice(&a).to_string());

        let a = vec![1, 2, 3, 4, 5, 6];
        assert_eq!("[1,2,3,4,..,6]", DisplaySlice(&a).to_string());

        let a = vec![1, 2, 3, 4, 5, 6, 7];
        assert_eq!("[1,2,3,4,..,7]", DisplaySlice(&a).to_string());
    }
}
