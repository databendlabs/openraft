pub trait MessageSummary<M> {
    /// Return a string of a big message
    fn summary(&self) -> String;
}

impl<T> MessageSummary<T> for &[&T]
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
impl<T> MessageSummary<T> for &[T]
where T: MessageSummary<T>
{
    fn summary(&self) -> String {
        let u = self.iter().collect::<Vec<_>>();
        u.as_slice().summary()
    }
}

impl<T> MessageSummary<T> for Option<T>
where T: MessageSummary<T> + 'static
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

impl<T> MessageSummary<T> for Option<&T>
where T: MessageSummary<T> + 'static
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

    // With `single-term-leader`, log id is a two-element tuple
    #[cfg(not(feature = "single-term-leader"))]
    #[test]
    fn test_display() {
        use crate::MessageSummary;

        let lid = crate::testing::log_id(1, 2, 3);
        assert_eq!("T1-N2-3", lid.to_string());
        assert_eq!("T1-N2-3", lid.summary());
        assert_eq!("Some(T1-N2-3)", Some(&lid).summary());
        assert_eq!("Some(T1-N2-3)", Some(lid).summary());

        let slc = vec![lid, lid];
        assert_eq!("T1-N2-3,T1-N2-3", slc.as_slice().summary());

        let slc = vec![&lid, &lid];
        assert_eq!("T1-N2-3,T1-N2-3", slc.as_slice().summary());
    }
}
