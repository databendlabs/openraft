use std::fmt;

/// Implement `Display` for `&[T]` if T is `Display`.
///
/// It outputs at most `max` elements, excluding those from the 5th to the second-to-last one:
/// - `DisplaySlice { slice: &[1,2,3,4,5,6], max: 5 }` outputs: `"[1,2,3,4,..,6]"`.
pub(crate) struct DisplaySlice<'a, T: fmt::Display> {
    pub slice: &'a [T],
    pub max: usize,
}

impl<'a, T: fmt::Display> DisplaySlice<'a, T> {
    /// Set the maximum number of elements to display.
    #[allow(unused)]
    pub fn limit(mut self, max: usize) -> Self {
        self.max = max;
        self
    }
}

impl<T: fmt::Display> fmt::Display for DisplaySlice<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.slice.len();

        write!(f, "[")?;

        if len > self.max {
            for (i, t) in self.slice[..(self.max - 1)].iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }

            write!(f, ",..,")?;
            write!(f, "{}", self.slice.last().unwrap())?;
        } else {
            for (i, t) in self.slice.iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }
        }

        write!(f, "]")
    }
}

pub(crate) trait DisplaySliceExt<'a, T: fmt::Display> {
    fn display(&'a self) -> DisplaySlice<'a, T>;

    /// Display at most `max` elements.
    fn display_n(&'a self, max: usize) -> DisplaySlice<'a, T> {
        self.display().limit(max)
    }
}

impl<T> DisplaySliceExt<'_, T> for [T]
where T: fmt::Display
{
    fn display(&self) -> DisplaySlice<'_, T> {
        DisplaySlice { slice: self, max: 5 }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_display_slice() {
        use crate::display_ext::DisplaySliceExt;

        let a = [1, 2, 3, 4];
        assert_eq!("[1,2,3,4]", a.display().to_string());

        let a = [1, 2, 3, 4, 5];
        assert_eq!("[1,2,3,4,5]", a.display().to_string());

        let a = [1, 2, 3, 4, 5, 6];
        assert_eq!("[1,2,3,4,..,6]", a.display().to_string());

        let a = [1, 2, 3, 4, 5, 6, 7];
        assert_eq!("[1,2,3,4,..,7]", a.display().to_string());

        let a = [1, 2, 3, 4, 5, 6, 7];
        assert_eq!("[1,..,7]", a.display().limit(2).to_string());
    }

    #[test]
    fn test_display_slice_limit() {
        use crate::display_ext::DisplaySliceExt;

        let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        // Default max: 5
        assert_eq!("[1,2,3,4,..,10]", a.display().to_string());

        // Change max to 3
        assert_eq!("[1,2,..,10]", a.display().limit(3).to_string());

        // Change max to 8
        assert_eq!("[1,2,3,4,5,6,7,..,10]", a.display().limit(8).to_string());

        // Change max to larger than length
        assert_eq!("[1,2,3,4,5,6,7,8,9,10]", a.display().limit(20).to_string());
    }

    #[test]
    fn test_display_slice_display_n() {
        use crate::display_ext::DisplaySliceExt;

        let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        // display_n should use the default implementation
        assert_eq!("[1,2,..,10]", a.display_n(3).to_string());
        assert_eq!("[1,2,3,4,5,6,7,..,10]", a.display_n(8).to_string());
        assert_eq!("[1,2,3,4,5,6,7,8,9,10]", a.display_n(20).to_string());
    }
}
