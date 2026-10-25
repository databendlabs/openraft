use std::error::Error;

use anyerror::AnyError;
use openraft_macros::since;

use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::ErrorSource;
use crate::errors::LearnerNotFound;
use crate::errors::NodeNotFound;
use crate::errors::UnprovenReadWriteIntersection;
use crate::node::NodeId;
use crate::vote::RaftCommittedLeaderId;

/// Errors from applying a membership edit or validating its target.
#[since(
    version = "0.10.0",
    change = "added `UserDefined` variant and error type parameter `E`"
)]
#[since(version = "0.10.0", change = "added `UnprovenReadWriteIntersection` variant")]
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(serialize = "E: serde::Serialize", deserialize = "E: serde::Deserialize<'de>"))
)]
pub enum MembershipError<NID, E = AnyError>
where
    NID: NodeId,
    E: Error + 'static,
{
    /// The membership configuration is empty.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A required node was not found.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<NID>),

    /// OpenRaft cannot prove that every read quorum intersects every write quorum.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    UnprovenReadWriteIntersection(#[from] UnprovenReadWriteIntersection),

    /// A membership edit rejected the target for a reason of its own.
    #[since(version = "0.10.0")]
    #[error("user-defined membership error: {0}")]
    UserDefined(E),
}

impl<NID, E> MembershipError<NID, E>
where
    NID: NodeId,
    E: Error + 'static,
{
    /// Maps a user-defined error while preserving structural validation errors.
    #[since(version = "0.10.0")]
    pub fn map_user_defined<T, F>(self, map: F) -> MembershipError<NID, T>
    where
        T: Error + 'static,
        F: FnOnce(E) -> T,
    {
        match self {
            Self::EmptyMembership(error) => MembershipError::EmptyMembership(error),
            Self::NodeNotFound(error) => MembershipError::NodeNotFound(error),
            Self::UnprovenReadWriteIntersection(error) => MembershipError::UnprovenReadWriteIntersection(error),
            Self::UserDefined(error) => {
                let error = map(error);
                MembershipError::UserDefined(error)
            }
        }
    }
}

/// Translate a membership validation failure into a rejected change request.
///
/// The two enums overlap, but they describe different layers: [`MembershipError`] says a
/// membership value is invalid, while [`ChangeMembershipError`] says a caller's request was
/// refused. Embedding one in the other would collapse that distinction and, worse, lose the
/// specific advice below.
///
/// `NodeNotFound` deliberately becomes `LearnerNotFound`. The only way membership validation
/// reports a missing node is a voter with no `Node` entry, which means the caller tried to promote
/// a node it never added. `LearnerNotFound` says exactly that, whereas `NodeNotFound` would only
/// say the node is missing. The `operation` field dropped in the process is always
/// `Operation::None` on this path, so nothing diagnostic is lost.
impl<CLID, NID, E> From<MembershipError<NID, E>> for ChangeMembershipError<CLID, NID, E>
where
    CLID: RaftCommittedLeaderId,
    E: ErrorSource,
    NID: NodeId,
{
    fn from(me: MembershipError<NID, E>) -> Self {
        match me {
            MembershipError::EmptyMembership(e) => ChangeMembershipError::EmptyMembership(e),
            MembershipError::NodeNotFound(e) => {
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: e.node_id })
            }
            MembershipError::UnprovenReadWriteIntersection(e) => {
                ChangeMembershipError::UnprovenReadWriteIntersection(e)
            }
            MembershipError::UserDefined(e) => ChangeMembershipError::UserDefined(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use anyerror::AnyError;

    use super::MembershipError;
    use crate::errors::EmptyMembership;
    use crate::errors::NodeNotFound;
    use crate::errors::Operation;
    use crate::membership::errors::UnprovenReadWriteIntersection;

    #[test]
    fn map_user_defined_wraps_native_error() {
        let source = io::Error::other("edit rejected");
        let expected = AnyError::new(&source);
        let expected = MembershipError::UserDefined(expected);
        let error = MembershipError::<u64, _>::UserDefined(source);
        let actual = error.map_user_defined(|source| AnyError::new(&source));
        assert_eq!(expected, actual);
    }

    #[test]
    fn map_user_defined_preserves_structural_errors() {
        let missing = NodeNotFound::new(3, Operation::None);
        let unproven = UnprovenReadWriteIntersection {
            membership: "custom membership".into(),
        };
        let errors: [MembershipError<u64>; 3] = [
            MembershipError::EmptyMembership(EmptyMembership {}),
            MembershipError::NodeNotFound(missing),
            MembershipError::UnprovenReadWriteIntersection(unproven),
        ];
        for expected in errors {
            let native = expected.clone();
            let native = native.map_user_defined(|_| io::Error::other("unexpected user-defined error"));
            let actual: MembershipError<u64> = native.map_user_defined(|_| panic!("structural error was wrapped"));
            assert_eq!(expected, actual);
        }
    }
}
