pub trait TryAsRef<T> {
    fn try_as_ref(&self) -> Option<&T>;
}
