use std::io;

use bincode::Options;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A versioned application-owned envelope; no OpenRaft type needs rkyv derives.
pub(crate) struct Codec;

impl Codec {
    const VERSION: u8 = 1;
    const MAX_FRAME: u64 = 8 * 1024 * 1024;

    pub(crate) fn encode<T>(value: &T) -> io::Result<Vec<u8>>
    where T: Serialize {
        let mut frame = vec![Self::VERSION];
        let body = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(Self::MAX_FRAME)
            .serialize(value)
            .map_err(io::Error::other)?;
        frame.extend_from_slice(&body);
        Ok(frame)
    }

    pub(crate) fn decode<T>(frame: &[u8]) -> io::Result<T>
    where T: DeserializeOwned {
        let Some((&Self::VERSION, body)) = frame.split_first() else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported wire version"));
        };
        if body.len() as u64 > Self::MAX_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
        }
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(Self::MAX_FRAME)
            .reject_trailing_bytes()
            .deserialize(body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub(crate) fn round_trip<T>(value: &T) -> io::Result<T>
    where T: Serialize + DeserializeOwned {
        Self::decode(&Self::encode(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::Codec;

    #[test]
    fn reject_wrong_version_truncation_and_trailing_bytes() -> anyhow::Result<()> {
        let mut frame = Codec::encode(&42_u64)?;
        assert_eq!(42, Codec::decode::<u64>(&frame)?);
        frame[0] = 2;
        assert!(Codec::decode::<u64>(&frame).is_err());
        frame[0] = 1;
        assert!(Codec::decode::<u64>(&frame[..frame.len() - 1]).is_err());
        frame.push(0);
        assert!(Codec::decode::<u64>(&frame).is_err());
        Ok(())
    }
}
