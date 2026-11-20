struct Foo {
    /// Since: 1.0.0
    a: u8,
    /// Since: 1.1.0
    b: u8,
    /// Since: 1.2.0
    c: u8,
    /// Since: 1.3.0
    d: u8,
    /// Since: 1.4.0
    #[clippy::since]
    e: u8,
}
fn main() {}
