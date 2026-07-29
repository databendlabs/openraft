#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
enum Name {
    Unit,
    Empty(),
}

fn main() {}
