/// Doc
///
/// By default, `Send` bounds will be added to the trait and to the return bounds of any async
/// functions defined within the trait.
///
/// If the `singlethreaded` feature is enabled, the trait definition remains the same without any
/// added `Send` bounds.
///
/// # Example
///
/// - list
///
/// Since: 1.0.0, Date(2021-01-01)
const A: i32 = 0;
