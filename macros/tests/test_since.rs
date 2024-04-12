use openraft_macros::since;

/// Doc
///
/// By default, `Send` bounds will be added to the trait and to the return bounds of any async
/// functions defined withing the trait.
///
/// If the `singlethreaded` feature is enabled, the trait definition remains the same without any
/// added `Send` bounds.
///
/// # Example
///
/// - list
#[since(version = "1.0.0", date = "2021-01-01")]
#[allow(dead_code)]
const A: i32 = 0;

#[since(version = "1.0.0")]
#[allow(dead_code)]
fn foo() {}

/*

#[since(version = "1.0.0..0")]
fn bad_semver() {}

#[since(version = "1.0.0", date = "2021-01--")]
fn bad_date() {}

 */
