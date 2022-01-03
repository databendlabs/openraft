pub trait MessageSummary {
    /// Return a string of a big message
    fn summary(&self) -> String;
}
