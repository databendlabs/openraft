use crate::pb;
use crate::typ::*;

impl From<pb::ClientWriteResponse> for ClientWriteResponse {
    fn from(r: pb::ClientWriteResponse) -> Self {
        ClientWriteResponse {
            log_id: r.log_id.unwrap().into(),
            data: r.data.unwrap(),
            membership: r.membership.map(|mem| mem.into()),
        }
    }
}

impl From<ClientWriteResponse> for pb::ClientWriteResponse {
    fn from(r: ClientWriteResponse) -> Self {
        pb::ClientWriteResponse {
            log_id: Some(r.log_id.into()),
            data: Some(r.data),
            membership: r.membership.map(|mem| mem.into()),
        }
    }
}
