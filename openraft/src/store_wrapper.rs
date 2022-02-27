use crate::RaftStorage;
use crate::RaftTypeConfig;

/// A wrapper extends the APIs of a base RaftStore.
pub trait Wrapper<C, T>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
{
    fn inner(&mut self) -> &mut T;
}
