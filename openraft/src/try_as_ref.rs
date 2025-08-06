/// Similar to [`AsRef<T>`], it does a cheap reference-to-reference conversion, but with the ability
/// to return `None` if it is unable to perform the conversion to `&T`.
pub trait TryAsRef<T> {
    /// Try to convert this type into a shared reference of `T`.
    fn try_as_ref(&self) -> Option<&T>;
}
