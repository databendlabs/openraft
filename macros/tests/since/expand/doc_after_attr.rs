// The `Since:` line goes after the last doc comment, even when a non-doc attribute comes before or
// between the doc comments.

/// Doc A.
#[allow(dead_code)]
/// Doc B.
#[openraft_macros::since(version = "1.0.0")]
struct Foo;

#[openraft_macros::since]
struct Bar {
    #[allow(dead_code)]
    /// Field doc.
    #[openraft_macros::since(version = "1.1.0")]
    a: i32,
}
