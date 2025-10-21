#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2024::*;
#[macro_use]
extern crate std;
fn main() {
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where
        T: Send;
}
