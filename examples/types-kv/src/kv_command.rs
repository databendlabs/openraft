use crate::DedupRequest;
use crate::Request;
use crate::RequestId;

/// A command that a KV state machine applies: a [`Request`], and the [`RequestId`] under which to
/// apply it at most once, if any.
pub trait KvCommand {
    /// The request to apply.
    fn request(&self) -> &Request;

    /// The ID under which the request is applied at most once, or `None` to apply the request each
    /// time it is committed.
    fn request_id(&self) -> Option<RequestId>;
}

impl KvCommand for Request {
    fn request(&self) -> &Request {
        self
    }

    fn request_id(&self) -> Option<RequestId> {
        None
    }
}

impl KvCommand for DedupRequest {
    fn request(&self) -> &Request {
        &self.request
    }

    fn request_id(&self) -> Option<RequestId> {
        Some(self.id)
    }
}
