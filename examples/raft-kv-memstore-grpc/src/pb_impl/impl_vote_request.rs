use crate::pb;
use crate::typ::VoteRequest;

impl From<VoteRequest> for pb::VoteRequest {
    fn from(vote_req: VoteRequest) -> Self {
        pb::VoteRequest {
            vote: Some(vote_req.vote),
            last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
            leadership_transfer: vote_req.leadership_transfer,
        }
    }
}

impl From<pb::VoteRequest> for VoteRequest {
    fn from(proto_vote_req: pb::VoteRequest) -> Self {
        let vote = proto_vote_req.vote.unwrap();
        let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
        VoteRequest {
            vote,
            last_log_id,
            leadership_transfer: proto_vote_req.leadership_transfer,
        }
    }
}
