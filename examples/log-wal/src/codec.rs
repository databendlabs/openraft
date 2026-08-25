//! Codec wrappers that give openraft types the `codeq::Codec` impl that
//! `raft_log` requires.
//!
//! `raft_log` writes every value through `codeq::Encode` and reads it back
//! through `codeq::Decode`. openraft types do not implement those traits, and
//! neither crate can add the impl, so this module wraps the values in local
//! types.
//!
//! The wrappers encode with MessagePack. A `raft_log` record sits in a stream
//! that holds more records after it, so the decoder must stop at the end of one
//! value instead of reading to the end of the stream. MessagePack is
//! self-delimiting and does stop there.

use std::any::type_name;
use std::cmp::Ordering;
use std::io;

use openraft::RaftTypeConfig;
use openraft::alias::VoteOf;
use openraft::vote::RaftVote;
use raft_log::codeq::Decode;
use raft_log::codeq::Encode;
use raft_log::codeq::OffsetWriter;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Codec wrapper that adds `Encode` and `Decode` to a foreign type.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MsgPack<T>(pub T);

impl<T> Encode for MsgPack<T>
where T: Serialize
{
    fn encode<W: io::Write>(&self, w: W) -> Result<usize, io::Error> {
        encode_msgpack(&self.0, w)
    }
}

impl<T> Decode for MsgPack<T>
where T: DeserializeOwned
{
    fn decode<R: io::Read>(r: R) -> Result<Self, io::Error> {
        let value = decode_msgpack(r)?;
        Ok(MsgPack(value))
    }
}

/// Codec wrapper for the vote.
///
/// The vote needs its own wrapper because `raft_log::Types::Vote` requires
/// `PartialOrd`, and openraft's `RaftVote` does not have `PartialOrd` as a
/// supertrait. `RaftVote::partial_cmp` provides the same order, so this wrapper
/// forwards to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgPackVote<C: RaftTypeConfig>(pub VoteOf<C>);

impl<C: RaftTypeConfig> PartialOrd for MsgPackVote<C> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        RaftVote::partial_cmp(&self.0, &other.0)
    }
}

impl<C: RaftTypeConfig> Encode for MsgPackVote<C> {
    fn encode<W: io::Write>(&self, w: W) -> Result<usize, io::Error> {
        encode_msgpack(&self.0, w)
    }
}

impl<C: RaftTypeConfig> Decode for MsgPackVote<C> {
    fn decode<R: io::Read>(r: R) -> Result<Self, io::Error> {
        let vote = decode_msgpack(r)?;
        Ok(MsgPackVote(vote))
    }
}

/// Encode one value as MessagePack and return the number of bytes written.
fn encode_msgpack<T, W>(value: &T, mut w: W) -> Result<usize, io::Error>
where
    T: Serialize,
    W: io::Write,
{
    let mut offset_writer = OffsetWriter::new(&mut w);

    rmp_serde::encode::write_named(&mut offset_writer, value).map_err(|e| {
        let msg = format!("{e}; when:(encode {})", type_name::<T>());
        io::Error::new(io::ErrorKind::InvalidData, msg)
    })?;

    Ok(offset_writer.offset())
}

/// Decode one MessagePack value, leaving the rest of the stream untouched.
fn decode_msgpack<T, R>(r: R) -> Result<T, io::Error>
where
    T: DeserializeOwned,
    R: io::Read,
{
    rmp_serde::decode::from_read(r).map_err(|e| {
        // An incomplete record at the tail of the WAL surfaces as a read
        // error. `raft_log` distinguishes it from corrupt data by the error
        // kind, so the kind is carried over instead of being flattened.
        let kind = match &e {
            rmp_serde::decode::Error::InvalidMarkerRead(io_err) => io_err.kind(),
            rmp_serde::decode::Error::InvalidDataRead(io_err) => io_err.kind(),
            _ => io::ErrorKind::InvalidData,
        };

        let msg = format!("{e}; when:(decode {})", type_name::<T>());
        io::Error::new(kind, msg)
    })
}
