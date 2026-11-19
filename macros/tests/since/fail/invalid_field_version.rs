#[openraft_macros::since]
pub struct Config {
    /// Valid.
    #[openraft_macros::since(version = "1.0.0")]
    pub a: u64,

    /// Invalid.
    #[openraft_macros::since(version = "1.x")]
    pub b: u64,
}

fn main() {
    let _ = Config { a: 1, b: 2 }.b;
}
