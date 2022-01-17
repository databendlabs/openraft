use crate::AppData;
use crate::AppDataResponse;
use crate::RaftStorage;

/// A wrapper extends the APIs of a base RaftStore.
pub trait Wrapper<D, R, T>
where
    D: AppData,
    R: AppDataResponse,
    T: RaftStorage<D, R>,
{
    fn inner(&self) -> &T;
}
