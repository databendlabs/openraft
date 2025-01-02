/// A trait for types whose order can be determined by a key.
///
/// Types implementing this trait define how they should be compared by providing a key
/// that implements [`PartialOrd`].
///
/// OpenRaft uses this trait to compare types that may not be [`PartialOrd`] themselves.
///
/// # Type Parameters
/// - `Key<'k>`: The type of the comparison key, which must be partially ordered and must not out
///   live the value.
///
/// # Examples
/// ```rust,ignore
/// # use openraft::base::ord_by::OrdBy;
///
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// impl OrdBy<()> for Person {
///     type By<'k> = &'k str;
///
///     fn ord_by(&self) -> Self::By<'_> {
///         &self.name
///     }
/// }
/// ```
pub(crate) trait OrdBy<C> {
    /// The key type used for comparison.
    type By<'k>: PartialOrd + 'k
    where Self: 'k;

    /// Returns the key used for comparing this value.
    fn ord_by(&self) -> Self::By<'_>;
}
