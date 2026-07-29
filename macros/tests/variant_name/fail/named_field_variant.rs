#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
enum Name {
    Unit,
    Named { a: u32 },
}

fn main() {}
