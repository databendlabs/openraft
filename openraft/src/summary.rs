pub trait MessageSummary {
    /// Return a string of a big message
    fn summary(&self) -> String;
}

impl<T> MessageSummary for &[T]
where T: MessageSummary
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

impl<T> MessageSummary for Option<T>
where T: MessageSummary
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
