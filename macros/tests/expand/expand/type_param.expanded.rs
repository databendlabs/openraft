struct S;
impl Tr for S {
    type Responder<T> = crate::impls::OneshotResponder<Self, T> where T: Send;
}
fn main() {}
