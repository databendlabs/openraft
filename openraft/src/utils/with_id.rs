use std::ops::Deref;
use std::ops::DerefMut;

use crate::replication::request_id::RequestId;

#[derive(Clone)]
pub(crate) struct WithId<T> {
    /// The id of this replication request.
    request_id: RequestId,
    inner: T,
}

impl<T> Deref for WithId<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for WithId<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> WithId<T> {
    pub(crate) fn new(request_id: RequestId, data: T) -> Self {
        Self {
            request_id,
            inner: data,
        }
    }

    pub(crate) fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub(crate) fn inner(&self) -> &T {
        &self.inner
    }

    pub(crate) fn into_inner(self) -> T {
        self.inner
    }
}
