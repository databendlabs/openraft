use std::fmt;

use openraft::entry::RaftPayload;
use serde::Deserialize;
use serde::Serialize;

use crate::Membership;
use crate::archive_buffer::ArchiveBuffer;

/// Only normal application data is archived. Raft metadata remains native.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Payload {
    Blank,
    Normal(ArchiveBuffer),
    Membership(Membership),
}

impl fmt::Display for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => write!(f, "blank"),
            Self::Normal(data) => data.fmt(f),
            Self::Membership(membership) => write!(f, "membership:{membership}"),
        }
    }
}

impl RaftPayload for Payload {
    type D = ArchiveBuffer;
    type Membership = Membership;

    fn blank() -> Self {
        Self::Blank
    }

    fn with_normal(self, data: Self::D) -> Self {
        Self::Normal(data)
    }

    fn with_membership(self, membership: Membership) -> Self {
        Self::Membership(membership)
    }

    fn get_membership(&self) -> Option<Membership> {
        match self {
            Self::Membership(membership) => Some(membership.clone()),
            _ => None,
        }
    }
}
