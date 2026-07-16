use hyper::StatusCode;
use openraft::NodeId;
use openraft::RaftTypeConfig;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::Infallible;
use openraft::errors::InitializeError;
use openraft::errors::LinearizableReadError;
use openraft::vote::RaftCommittedLeaderId;

use crate::client::FollowerReadError;

pub trait HttpStatus {
    fn status_code(&self) -> StatusCode;
}

impl<T, E> HttpStatus for Result<T, E>
where E: HttpStatus
{
    fn status_code(&self) -> StatusCode {
        match self {
            Ok(_) => StatusCode::OK,
            Err(e) => e.status_code(),
        }
    }
}

impl HttpStatus for Infallible {
    fn status_code(&self) -> StatusCode {
        match *self {}
    }
}

impl<C> HttpStatus for ClientWriteError<C>
where C: RaftTypeConfig
{
    fn status_code(&self) -> StatusCode {
        match self {
            ClientWriteError::ForwardToLeader(_) => StatusCode::SERVICE_UNAVAILABLE,
            ClientWriteError::ChangeMembershipError(e) => e.status_code(),
        }
    }
}

impl<CLID, NID> HttpStatus for ChangeMembershipError<CLID, NID>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
{
    fn status_code(&self) -> StatusCode {
        match self {
            ChangeMembershipError::InProgress(_) => StatusCode::CONFLICT,
            ChangeMembershipError::EmptyMembership(_) | ChangeMembershipError::LearnerNotFound(_) => {
                StatusCode::BAD_REQUEST
            }
        }
    }
}

impl<C> HttpStatus for InitializeError<C>
where C: RaftTypeConfig
{
    fn status_code(&self) -> StatusCode {
        match self {
            InitializeError::NotAllowed(_) => StatusCode::CONFLICT,
            InitializeError::NotInMembers(_) => StatusCode::BAD_REQUEST,
        }
    }
}

impl<C> HttpStatus for LinearizableReadError<C>
where C: RaftTypeConfig
{
    fn status_code(&self) -> StatusCode {
        match self {
            LinearizableReadError::ForwardToLeader(_) | LinearizableReadError::QuorumNotEnough(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
        }
    }
}

impl HttpStatus for FollowerReadError {
    fn status_code(&self) -> StatusCode {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
