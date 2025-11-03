use std::fmt;

/// A container that ensures values only increase monotonically.
///
/// Updates are rejected if the new value is not greater than the current value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub(crate) struct MonotonicIncrease<T>
where T: PartialOrd + fmt::Debug
{
    value: Option<T>,
}

impl<T> MonotonicIncrease<T>
where T: PartialOrd + fmt::Debug
{
    /// Creates a new monotonic container with an optional initial value.
    #[allow(dead_code)]
    pub(crate) fn new(value: Option<T>) -> Self {
        Self { value }
    }

    /// Returns a reference to the current value.
    pub(crate) fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Attempts to update the value if it's greater than the current value.
    ///
    /// Returns `Ok(Option<T>)` with the old value on success,
    /// or `Err(Option<&T>)` with a reference to the current stored value if rejected.
    pub(crate) fn try_update(&mut self, new_value: T) -> Result<Option<T>, Option<&T>> {
        self.try_update_option(Some(new_value))
    }

    /// Attempts to update with an optional value.
    ///
    /// Compares `new_value` with the stored value. If greater, updates and returns the old value.
    /// Otherwise returns `Err(Option<&T>)` with a reference to the current stored value.
    pub(crate) fn try_update_option(&mut self, new_value: Option<T>) -> Result<Option<T>, Option<&T>> {
        if new_value > self.value {
            Ok(std::mem::replace(&mut self.value, new_value))
        } else {
            Err(self.value.as_ref())
        }
    }

    /// Consumes the container and returns the inner value.
    #[allow(dead_code)]
    pub(crate) fn into_inner(self) -> Option<T> {
        self.value
    }
}

impl<T> Default for MonotonicIncrease<T>
where T: PartialOrd + fmt::Debug
{
    fn default() -> Self {
        Self { value: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_value() {
        let m = MonotonicIncrease::new(None::<i32>);
        assert_eq!(m.value(), None);

        let m = MonotonicIncrease::new(Some(5));
        assert_eq!(m.value(), Some(&5));
    }

    #[test]
    fn test_default() {
        let m: MonotonicIncrease<i32> = MonotonicIncrease::default();
        assert_eq!(m.value(), None);
    }

    #[test]
    fn test_try_update_from_none() {
        let mut m = MonotonicIncrease::new(None);
        assert_eq!(m.try_update(10), Ok(None));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_try_update_increasing() {
        let mut m = MonotonicIncrease::new(Some(5));
        assert_eq!(m.try_update(10), Ok(Some(5)));
        assert_eq!(m.value(), Some(&10));

        assert_eq!(m.try_update(20), Ok(Some(10)));
        assert_eq!(m.value(), Some(&20));
    }

    #[test]
    fn test_try_update_equal_rejected() {
        let mut m = MonotonicIncrease::new(Some(10));
        assert_eq!(m.try_update(10), Err(Some(&10)));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_try_update_decreasing_rejected() {
        let mut m = MonotonicIncrease::new(Some(10));
        assert_eq!(m.try_update(5), Err(Some(&10)));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_into_inner() {
        let m = MonotonicIncrease::new(Some(42));
        assert_eq!(m.into_inner(), Some(42));

        let m = MonotonicIncrease::new(None::<i32>);
        assert_eq!(m.into_inner(), None);
    }

    #[test]
    fn test_try_update_option_with_none() {
        let mut m = MonotonicIncrease::new(Some(10));
        assert_eq!(m.try_update_option(None), Err(Some(&10)));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_try_update_option_from_none() {
        let mut m = MonotonicIncrease::new(None);
        assert_eq!(m.try_update_option(Some(10)), Ok(None));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_try_update_option_increasing() {
        let mut m = MonotonicIncrease::new(Some(5));
        assert_eq!(m.try_update_option(Some(10)), Ok(Some(5)));
        assert_eq!(m.value(), Some(&10));

        assert_eq!(m.try_update_option(Some(20)), Ok(Some(10)));
        assert_eq!(m.value(), Some(&20));
    }

    #[test]
    fn test_try_update_option_equal_rejected() {
        let mut m = MonotonicIncrease::new(Some(10));
        assert_eq!(m.try_update_option(Some(10)), Err(Some(&10)));
        assert_eq!(m.value(), Some(&10));
    }

    #[test]
    fn test_try_update_option_decreasing_rejected() {
        let mut m = MonotonicIncrease::new(Some(10));
        assert_eq!(m.try_update_option(Some(5)), Err(Some(&10)));
        assert_eq!(m.value(), Some(&10));
    }
}
