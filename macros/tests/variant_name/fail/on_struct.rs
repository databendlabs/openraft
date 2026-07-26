#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
struct NotAnEnum {
    a: u32,
}

fn main() {}
