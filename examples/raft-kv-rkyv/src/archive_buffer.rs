use std::fmt;
use std::io;
use std::sync::Arc;

use rkyv::rancor::Error;
use rkyv::util::AlignedVec;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

use crate::command::ArchivedCommand;
use crate::command::Command;

/// An owning archive handle, not a borrowed `&ArchivedCommand` with an invented lifetime.
/// Every construction path validates bytes in their final, aligned allocation.
#[derive(Clone, Debug)]
pub(crate) struct ArchiveBuffer(Arc<AlignedVec>);

impl ArchiveBuffer {
    pub(crate) fn new(key: &str, value: &str) -> io::Result<Self> {
        let command = Command {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let bytes = rkyv::to_bytes::<Error>(&command).map_err(io::Error::other)?;
        Self::validated(bytes)
    }

    fn validated(bytes: AlignedVec) -> io::Result<Self> {
        rkyv::access::<ArchivedCommand, Error>(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self(Arc::new(bytes)))
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut aligned = AlignedVec::with_capacity(bytes.len());
        aligned.extend_from_slice(bytes);
        Self::validated(aligned)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn archived(&self) -> &ArchivedCommand {
        // Safe access checks the archive again; it never reconstructs Command or its Strings.
        rkyv::access::<ArchivedCommand, Error>(&self.0).expect("validated immutable archive")
    }
}

impl fmt::Display for ArchiveBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rkyv:{} bytes", self.bytes().len())
    }
}

impl Serialize for ArchiveBuffer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        // Serde frames the already archived bytes; it does not traverse ArchivedCommand.
        serializer.serialize_bytes(self.bytes())
    }
}

impl<'de> Deserialize<'de> for ArchiveBuffer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::ArchiveBuffer;

    #[test]
    fn clone_shares_archive_and_wire_decode_owns_a_new_archive() -> anyhow::Result<()> {
        let original = ArchiveBuffer::new("key", "value")?;
        let cloned = original.clone();
        assert_eq!(original.bytes().as_ptr(), cloned.bytes().as_ptr());

        let frame = bincode::serialize(&original)?;
        let received: ArchiveBuffer = bincode::deserialize(&frame)?;
        assert_ne!(original.bytes().as_ptr(), received.bytes().as_ptr());
        assert_eq!(original.bytes(), received.bytes());
        assert_eq!("value", received.archived().value.as_str());
        Ok(())
    }

    #[test]
    fn reject_invalid_archives_at_the_wire_boundary() -> anyhow::Result<()> {
        let valid = ArchiveBuffer::new(&"key".repeat(64), &"value".repeat(64))?;
        let mut invalid_utf8 = valid.bytes().to_vec();
        // Long strings are stored out of line, beginning with the key's UTF-8 bytes.
        invalid_utf8[0] = 0xff;
        for bytes in [vec![], vec![0xff; 3], invalid_utf8] {
            assert!(ArchiveBuffer::from_bytes(&bytes).is_err());
            let frame = bincode::serialize(&bytes)?;
            assert!(bincode::deserialize::<ArchiveBuffer>(&frame).is_err());
        }
        assert!(ArchiveBuffer::from_bytes(&valid.bytes()[..valid.bytes().len() - 1]).is_err());
        Ok(())
    }
}
