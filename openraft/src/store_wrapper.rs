use crate::RaftConfig;
use crate::RaftStorage;

/// A wrapper extends the APIs of a base RaftStore.
pub trait Wrapper<C, T>
where
    C: RaftConfig,
    T: RaftStorage<C>,
{
    fn inner(&mut self) -> &mut T;
}
