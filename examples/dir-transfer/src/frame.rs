//! Wire frame types of the directory transfer protocol.

use std::io;

use crc::CRC_64_XZ;
use crc::Crc;
use serde::Deserialize;
use serde::Serialize;

/// The protocol version this crate emits and accepts.
///
/// Version 1 fixes the per-file checksum to CRC-64/XZ.
pub const FORMAT_VERSION: u32 = 1;

/// Longest accepted manifest file name, in bytes.
pub const MAX_NAME_LEN: usize = 255;

/// Largest accepted [`DirFrame::Chunk`] payload, in bytes.
pub const MAX_CHUNK_SIZE: usize = 8 * 1024 * 1024;

static CRC64: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

/// Name and size of one file in a [`DirManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub struct FileMeta {
    /// Flat file name; no path separators.
    pub name: String,

    /// File size in bytes.
    pub size: u64,
}

/// Description of the complete transfer, sent before any file data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub struct DirManifest {
    /// Protocol version; see [`FORMAT_VERSION`].
    pub format_version: u32,

    /// Every file of the directory, in transfer order.
    pub files: Vec<FileMeta>,
}

/// One frame of the transfer stream.
///
/// A valid stream is `Manifest`, then for each manifest entry in manifest order `FileStart`,
/// zero or more `Chunk`s, `FileEnd`, and finally `End`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub enum DirFrame {
    /// Names and sizes of every file about to be transferred.
    Manifest(DirManifest),

    /// Begin the next manifest file.
    FileStart {
        /// Name of the file; must equal the next manifest entry.
        name: String,
    },

    /// A run of file bytes, at most [`MAX_CHUNK_SIZE`] long.
    Chunk {
        /// The bytes.
        data: Vec<u8>,
    },

    /// End of the current file.
    FileEnd {
        /// CRC-64/XZ checksum of the complete file contents.
        checksum: u64,
    },

    /// End of the stream; every manifest file has been transferred.
    End,
}

/// Return a streaming hasher for the per-file checksum of [`FORMAT_VERSION`] 1.
pub(crate) fn checksum_digest() -> crc::Digest<'static, u64> {
    CRC64.digest()
}

/// Validate a manifest file name before any filesystem access.
///
/// Names must be non-empty, at most [`MAX_NAME_LEN`] bytes, free of path separators and NUL, and
/// not `.` or `..`.
pub(crate) fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Err(invalid_data("empty file name"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(invalid_data(format!("file name longer than {MAX_NAME_LEN} bytes")));
    }
    if name == "." || name == ".." {
        return Err(invalid_data(format!("file name {name:?} is not allowed")));
    }
    if name.contains(['/', '\\', '\0']) {
        return Err(invalid_data(format!(
            "file name {name:?} contains a path separator or NUL"
        )));
    }
    Ok(())
}

/// Build an `InvalidData` error; the protocol maps every violation to this kind.
pub(crate) fn invalid_data(msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::checksum_digest;
    use super::validate_name;

    #[test]
    fn test_checksum_is_crc64_xz() {
        // CRC-64/XZ standard check value for "123456789".
        let mut digest = checksum_digest();
        digest.update(b"123456789");
        assert_eq!(0x995dc9bbdf1939fa, digest.finalize());

        // Streaming in parts equals hashing at once.
        let mut digest = checksum_digest();
        digest.update(b"1234");
        digest.update(b"56789");
        assert_eq!(0x995dc9bbdf1939fa, digest.finalize());
    }

    #[test]
    fn test_validate_name() {
        validate_name("CURRENT").unwrap();
        validate_name("000012.sst").unwrap();
        validate_name(&"x".repeat(super::MAX_NAME_LEN)).unwrap();

        let invalid = [
            "",
            ".",
            "..",
            "a/b",
            "/abs",
            "a\\b",
            "a\0b",
            &"x".repeat(super::MAX_NAME_LEN + 1),
        ];
        for name in invalid {
            let err = validate_name(name).unwrap_err();
            assert_eq!(io::ErrorKind::InvalidData, err.kind(), "name: {name:?}");
        }
    }
}
