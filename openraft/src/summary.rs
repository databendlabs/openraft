use std::fmt;

/// Convert a type `T` to string.
///
/// If `T` implements `Display`, then `T` implements `MessageSummary` too.
///
/// MessageSummary is also a handy tool for displaying `Option` and `Slice` because it is
/// implemented for:
/// - `Option<T: MessageSummary>`
/// - and `&[T]` where `T: MessageSummary`.
///
///
/// # Examples
/// ```rust,ignore
/// # use openraft::MessageSummary;
/// # use openraft::testing::log_id;
/// let lid = log_id(1, 2, 3);
/// assert_eq!("1-2-3", lid.to_string(), "LogId is Display");
/// assert_eq!("1-2-3", lid.summary(), "Thus LogId is also MessageSummary");
/// assert_eq!("Some(1-2-3)", Some(lid).summary(), "Option<LogId> can be displayed too");
/// assert_eq!("Some(1-2-3)", Some(&lid).summary(), "Option<&LogId> can be displayed too");
///
/// let slc = vec![lid, lid];
/// assert_eq!("1-2-3,1-2-3", slc.as_slice().summary(), "&[LogId] can be displayed too");
///
/// let slc = vec![&lid, &lid];
/// assert_eq!("1-2-3,1-2-3", slc.as_slice().summary(), "&[&LogId] can be displayed too");
/// ```
pub trait MessageSummary<M> {
    /// Return a string of a big message
    fn summary(&self) -> String;
}

impl<T> MessageSummary<T> for T
where T: fmt::Display
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<T> MessageSummary<T> for &[T]
where T: MessageSummary<T>
{
    fn summary(&self) -> String {
        if self.is_empty() {
            return "{}".to_string();
        }
        let mut res = Vec::with_capacity(self.len());
        if self.len() <= 5 {
            for x in self.iter() {
                let e = x.summary();
                res.push(e);
            }

            res.join(",")
        } else {
            let first = self.first().unwrap();
            let last = self.last().unwrap();

            format!("{} ... {}", first.summary(), last.summary())
        }
    }
}

impl<T> MessageSummary<T> for Option<T>
where T: MessageSummary<T>
{
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(x) => {
                format!("Some({})", x.summary())
            }
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_display() {
        use crate::MessageSummary;

        let lid = crate::engine::testing::log_id(1, 2, 3);
        assert_eq!("T1-N2.3", lid.to_string());
        assert_eq!("T1-N2.3", lid.summary());
        assert_eq!("Some(T1-N2.3)", Some(&lid).summary());
        assert_eq!("Some(T1-N2.3)", Some(lid).summary());

        let slc = vec![lid, lid];
        assert_eq!("T1-N2.3,T1-N2.3", slc.as_slice().summary());

        let slc = vec![&lid, &lid];
        assert_eq!("T1-N2.3,T1-N2.3", slc.as_slice().summary());
    }
}
