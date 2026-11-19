#[openraft_macros::add_async_trait]
pub struct NotATrait {
    pub a: u64,
}

fn main() {
    let _ = NotATrait { a: 1 }.a;
}
