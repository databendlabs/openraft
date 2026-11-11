mod impls {
    pub struct OneshotResponder<C, T>(std::marker::PhantomData<(C, T)>);
}
trait Tr {
    type Responder<T> where T: Send;
}
struct S;
impl Tr for S {
    type Responder<T> = crate::impls::OneshotResponder<Self, T> where T: Send;
}
fn main() {}
