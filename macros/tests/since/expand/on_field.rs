// `#[since]` on a container rewrites `#[since(...)]` markers on its fields / variants.

#[openraft_macros::since]
struct Foo {
    /// Field a.
    #[openraft_macros::since(version = "1.0.0")]
    a: i32,

    #[openraft_macros::since(version = "1.1.0", date = "2021-01-01")]
    b: i32,
}

#[openraft_macros::since(version = "2.0.0")]
enum Bar {
    /// Variant X.
    #[openraft_macros::since(version = "1.5.0")]
    X,
    Y,
}
