#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
enum Name {
    Unit,
    Pair(u32, u32),
}

fn main() {}
