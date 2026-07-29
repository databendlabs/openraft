#![allow(dead_code)]

#[derive(openraft_macros::VariantName)]
#[variant_name(suffix = "::")]
enum Name {
    Unit,
}

fn main() {}
