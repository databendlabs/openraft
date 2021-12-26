/// A wrapper extends the APIs of a base RaftStore.
pub trait Wrapper<T> // where
{
    fn inner(&self) -> &T;
}
