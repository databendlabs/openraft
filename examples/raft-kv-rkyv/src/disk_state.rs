use std::collections::BTreeMap;
use std::io;

use bincode::Options;
use openraft::alias::VoteOf;
use serde::Deserialize;
use serde::Serialize;

use crate::Entry;
use crate::LogId;
use crate::TypeConfig;

/// One complete, versioned disk generation. Application data remains archived bytes.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct DiskState {
    pub(crate) entries: BTreeMap<u64, Entry>,
    pub(crate) vote: Option<VoteOf<TypeConfig>>,
    pub(crate) committed: Option<LogId>,
    pub(crate) purged: Option<LogId>,
}

impl DiskState {
    // Changing native metadata, the Command schema, rkyv settings, or encoding requires a
    // new disk version (and an explicit migration). This is independent of the wire codec.
    const MAGIC: &[u8; 8] = b"ORKYV001";
    /// The magic followed by the CRC32 of the body.
    const HEADER_LEN: usize = Self::MAGIC.len() + 4;
    pub(crate) const MAX_BYTES: u64 = 64 * 1024 * 1024;

    pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
        let body = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(Self::MAX_BYTES - Self::HEADER_LEN as u64)
            .serialize(self)
            .map_err(io::Error::other)?;
        let mut bytes = Vec::with_capacity(Self::HEADER_LEN + body.len());
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < Self::HEADER_LEN || bytes.len() as u64 > Self::MAX_BYTES || !bytes.starts_with(Self::MAGIC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported or truncated disk image",
            ));
        }
        let (header, body) = bytes.split_at(Self::HEADER_LEN);
        if header[Self::MAGIC.len()..] != crc32fast::hash(body).to_le_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "disk image checksum mismatch",
            ));
        }
        let state: Self = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(Self::MAX_BYTES - Self::HEADER_LEN as u64)
            .reject_trailing_bytes()
            .deserialize(body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        // ArchiveBuffer's serde decoder validates each archive in its final aligned allocation.
        if state.entries.iter().any(|(index, entry)| *index != entry.log_id.index) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "entry index mismatch"));
        }
        Ok(state)
    }
}
