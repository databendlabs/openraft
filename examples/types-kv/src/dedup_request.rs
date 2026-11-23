use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::Request;
use crate::RequestId;

/// A [`Request`] tagged with the [`RequestId`] of the client write that carries it.
///
/// A state machine applies the request at most once per ID: a request whose ID it already applied
/// is answered with the response of that first application. A client can therefore resend a write
/// whose outcome it does not know, such as one whose response was lost.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DedupRequest {
    pub id: RequestId,
    pub request: Request,
}

impl DedupRequest {
    pub fn new(id: RequestId, request: Request) -> Self {
        DedupRequest { id, request }
    }
}

impl fmt::Display for DedupRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.id, self.request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_requests_keep_their_json_and_postcard_schema() {
        let json = r#"{"id":{"client_id":3,"serial":5},"request":{"Set":{"key":"k","value":"v"}}}"#;
        let binary = [3, 5, 0, 1, b'k', 1, b'v'];
        let json_request: DedupRequest = serde_json::from_str(json).unwrap();
        let binary_request: DedupRequest = postcard::from_bytes(&binary).unwrap();
        let actual_json = serde_json::to_string(&binary_request).unwrap();
        let actual_binary = postcard::to_allocvec(&json_request).unwrap();
        assert_eq!(json, actual_json);
        assert_eq!(binary.as_slice(), actual_binary);
    }
}
