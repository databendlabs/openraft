use std::fmt::Formatter;

use crate::alias::LogIdOf;
use crate::display_ext::DisplayOptionExt;
use crate::RaftTypeConfig;
use crate::ReadPolicy;

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct GetReadLogIdRequest {
    pub(crate) read_policy: ReadPolicy,
}

impl std::fmt::Display for GetReadLogIdRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "read policy: {}", self.read_policy)
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct GetReadLogIdResponse<C: RaftTypeConfig> {
    pub read_log_id: Option<LogIdOf<C>>,
    pub applied: Option<LogIdOf<C>>,
}

impl<C: RaftTypeConfig> GetReadLogIdResponse<C> {
    pub fn is_none(&self) -> bool {
        self.read_log_id.is_none() && self.applied.is_none()
    }
}

impl<C> From<GetReadLogIdResponse<C>> for (Option<LogIdOf<C>>, Option<LogIdOf<C>>)
where C: RaftTypeConfig
{
    fn from(value: GetReadLogIdResponse<C>) -> Self {
        (value.read_log_id, value.applied)
    }
}

impl<C> From<(Option<LogIdOf<C>>, Option<LogIdOf<C>>)> for GetReadLogIdResponse<C>
where C: RaftTypeConfig
{
    fn from(value: (Option<LogIdOf<C>>, Option<LogIdOf<C>>)) -> Self {
        Self {
            read_log_id: value.0,
            applied: value.1,
        }
    }
}

impl<C> std::fmt::Display for GetReadLogIdResponse<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "read_log_id: {}, applied: {}",
            self.read_log_id.display(),
            self.applied.display()
        )
    }
}
