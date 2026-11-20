// A marker may use the bare name, the crate path, or a re-export path in `openraft`; any other
// attribute, such as another tool's, stays on the field.

#[openraft_macros::since]
struct Foo {
    #[since(version = "1.0.0")]
    a: u8,

    #[openraft_macros::since(version = "1.1.0")]
    b: u8,

    #[openraft::macros::since(version = "1.2.0")]
    c: u8,

    #[openraft::openraft_macros::since(version = "1.3.0")]
    d: u8,

    #[clippy::since]
    #[openraft_macros::since(version = "1.4.0")]
    e: u8,
}

fn main() {}
