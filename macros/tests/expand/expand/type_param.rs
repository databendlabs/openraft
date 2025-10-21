fn main() {
    openraft_macros::expand!(
        !KEYED,
        (T, ATTR, V) => {ATTR type T = V;},
        (Responder<T>, , crate::impls::OneshotResponder<Self, T> where T: Send,),
    );
}
