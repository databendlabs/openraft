#![feature(prelude_import)]
#[macro_use]
extern crate std;
#[prelude_import]
use std::prelude::rust_2024::*;
fn main() {
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where
        T: Send;
}
