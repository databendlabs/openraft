/// A trait for types that can be compared via a key.
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
/// ```
/// # use openraft::base::cmp::CompareByKey;
///
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// impl CompareByKey<()> for Person {
///     type Key<'k> = &'k str;
///
///     fn cmp_key(&self) -> Self::Key<'_> {
///         &self.name
///     }
/// }
/// ```
pub(crate) trait CompareByKey<C> {
    /// The key type used for comparison.
    type Key<'k>: PartialOrd + 'k
    where Self: 'k;

    /// Returns the key used for comparing this value.
    fn cmp_key(&self) -> Self::Key<'_>;
}
