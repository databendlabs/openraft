#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
union NotAnEnum {
    a: u32,
}

fn main() {}
