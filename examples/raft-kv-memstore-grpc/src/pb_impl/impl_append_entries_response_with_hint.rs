use std::io;

use openraft::raft::AppendEntriesResponseWithHint;
use openraft::raft::ConflictHint;

use crate::TypeConfig;
use crate::pb;

impl TryFrom<pb::AppendEntriesResponseWithHint> for AppendEntriesResponseWithHint<TypeConfig> {
    type Error = io::Error;

    fn try_from(value: pb::AppendEntriesResponseWithHint) -> Result<Self, Self::Error> {
        let response = value.response.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing response in AppendEntriesWithHint reply",
            )
        })?;
        let hint = match value.hint {
            Some(hint) => {
                let log_ids = hint.log_ids.into_iter();
                let log_ids = log_ids.map(Into::into).collect();
                Some(ConflictHint::new(log_ids))
            }
            None => None,
        };
        let response = response.into();
        let response = Self::new(response, hint);
        Ok(response)
    }
}

impl From<AppendEntriesResponseWithHint<TypeConfig>> for pb::AppendEntriesResponseWithHint {
    fn from(value: AppendEntriesResponseWithHint<TypeConfig>) -> Self {
        let response = value.response.into();
        let hint = match value.hint {
            Some(hint) => {
                let log_ids = hint.log_ids.into_iter();
                let log_ids = log_ids.map(Into::into).collect();
                Some(pb::ConflictHint { log_ids })
            }
            None => None,
        };
        Self {
            response: Some(response),
            hint,
        }
    }
}

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;
    use crate::typ::AppendEntriesResponse;
    use crate::typ::LogId;

    #[test]
    fn hint_reply_round_trip() -> Result<(), io::Error> {
        let log_id = LogId::new(2, 5);
        let committed = LogId::new(1, 2);
        for log_ids in [vec![], vec![log_id], vec![committed, log_id]] {
            let hint = ConflictHint::new(log_ids);
            let expected = AppendEntriesResponseWithHint::new(AppendEntriesResponse::Conflict, Some(hint));
            let encoded = pb::AppendEntriesResponseWithHint::from(expected.clone());
            let bytes = encoded.encode_to_vec();
            let decoded = pb::AppendEntriesResponseWithHint::decode(bytes.as_slice())?;
            let actual = AppendEntriesResponseWithHint::try_from(decoded)?;
            assert_eq!(expected, actual);
        }
        Ok(())
    }

    #[test]
    fn legacy_conflict_wire_format_is_unchanged() {
        let response = pb::AppendEntriesResponse::from(AppendEntriesResponse::Conflict);
        let bytes = response.encode_to_vec();
        assert_eq!(vec![0x10, 0x01], bytes);
    }

    #[test]
    fn reply_requires_an_append_response() {
        let response = pb::AppendEntriesResponseWithHint {
            response: None,
            hint: None,
        };
        let error = AppendEntriesResponseWithHint::<TypeConfig>::try_from(response).unwrap_err();
        let kind = error.kind();
        assert_eq!(io::ErrorKind::InvalidData, kind);
    }
}
