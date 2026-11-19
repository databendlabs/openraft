use super::*;
use crate::profile::Profile;
use crate::wire_link::WireLink;

#[test]
fn cloned_positions_and_mutations_are_independent() -> io::Result<()> {
    let mut original = SnapshotData::new(b"abcd".to_vec());
    original.set_position(1);
    let mut other = original.clone();
    let shared = Arc::ptr_eq(&original.data, &other.data);
    assert!(shared);
    let mut byte = [0];
    Read::read_exact(&mut original, &mut byte)?;
    assert_eq!(*b"b", byte);
    let position = original.position();
    assert_eq!(2, position);
    let position = other.position();
    assert_eq!(1, position);

    Write::write_all(&mut other, b"Z")?;
    let value = other.get_ref();
    let value = value.as_slice();
    assert_eq!(b"aZcd", value);
    other.get_mut()[0] = b'Y';
    let value = other.get_ref();
    let value = value.as_slice();
    assert_eq!(b"YZcd", value);
    let value = original.get_ref();
    let value = value.as_slice();
    assert_eq!(b"abcd", value);
    let shared = Arc::ptr_eq(&original.data, &other.data);
    assert!(!shared);
    Ok(())
}

#[test]
fn owned_extraction_preserves_other_handles_and_unique_buffers() {
    let cached = SnapshotData::new(b"cached".to_vec());
    let output = cached.clone();
    let mut owned = output.into_inner();
    owned[0] = b'C';
    let value = cached.get_ref();
    let value = value.as_slice();
    assert_eq!(b"cached", value);
    let value = owned.as_slice();
    assert_eq!(b"Cached", value);
    let pointer = owned.as_ptr();
    let unique = SnapshotData::new(owned);
    let owned = unique.into_inner();
    let returned = owned.as_ptr();
    assert_eq!(pointer, returned);
    let value = owned.as_slice();
    assert_eq!(b"Cached", value);
}

#[test]
fn seek_holes_and_errors_preserve_cursor_semantics() -> io::Result<()> {
    let mut data = SnapshotData::new(b"abcd".to_vec());
    let position = Seek::seek(&mut data, SeekFrom::Start(6))?;
    assert_eq!(6, position);
    Write::write_all(&mut data, b"xy")?;
    let value = data.get_ref();
    let value = value.as_slice();
    assert_eq!(b"abcd\0\0xy", value);
    let error = Seek::seek(&mut data, SeekFrom::Current(-9)).unwrap_err();
    let kind = error.kind();
    assert_eq!(io::ErrorKind::InvalidInput, kind);
    let position = data.position();
    assert_eq!(8, position);
    data.set_position(7);
    let mut buffer = [0; 2];
    let error = Read::read_exact(&mut data, &mut buffer).unwrap_err();
    let kind = error.kind();
    assert_eq!(io::ErrorKind::UnexpectedEof, kind);
    let position = data.position();
    assert_eq!(8, position);
    data.set_position(u64::MAX);
    let read = Read::read(&mut data, &mut buffer)?;
    assert_eq!(0, read);
    let position = data.position();
    assert_eq!(u64::MAX, position);
    let value = data.get_ref();
    let value = value.as_slice();
    assert_eq!(b"abcd\0\0xy", value);
    Ok(())
}

#[tokio::test]
async fn async_receive_and_read_keep_cursor_and_buffer_independence() -> io::Result<()> {
    let mut received = SnapshotData::default();
    let empty = received.clone();
    tokio::io::AsyncWriteExt::write_all(&mut received, b"abc").await?;
    let position = tokio::io::AsyncSeekExt::seek(&mut received, SeekFrom::Start(6)).await?;
    assert_eq!(6, position);
    tokio::io::AsyncWriteExt::write_all(&mut received, b"z").await?;
    tokio::io::AsyncWriteExt::flush(&mut received).await?;
    tokio::io::AsyncWriteExt::shutdown(&mut received).await?;
    tokio::io::AsyncSeekExt::seek(&mut received, SeekFrom::Start(0)).await?;
    let other = received.clone();
    let mut value = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut received, &mut value).await?;
    let value = value.as_slice();
    assert_eq!(b"abc\0\0\0z", value);
    let position = other.position();
    assert_eq!(0, position);
    let position = received.position();
    assert_eq!(7, position);
    let empty = empty.into_inner();
    let expected = Vec::<u8>::new();
    assert_eq!(expected, empty);
    Ok(())
}

#[tokio::test]
async fn serialized_shared_bytes_keep_the_vec_wire_format_and_decode_independently() -> anyhow::Result<()> {
    let source = SnapshotData::new(b"serialized snapshot".to_vec());
    let shared = source.into_shared();
    let encoded = serde_json::to_vec(&shared)?;
    let expected = serde_json::to_vec(shared.as_ref())?;
    assert_eq!(expected, encoded);
    let profile = Arc::new(Profile {
        profile: true,
        serialized_transport: true,
        ..Profile::default()
    });
    let link = WireLink::new(profile, std::time::Duration::ZERO, 0, 1);
    let transmitted = link.transmit(shared.clone()).await?;
    let shared_allocation = Arc::ptr_eq(&shared, &transmitted);
    assert!(!shared_allocation);
    assert_eq!(shared, transmitted);
    let mut received = SnapshotData::from_shared(transmitted);
    Write::write_all(&mut received, b"S")?;
    let original = shared.as_ref();
    let original = original.as_slice();
    assert_eq!(b"serialized snapshot", original);
    let value = received.get_ref();
    let value = value.as_slice();
    assert_eq!(b"Serialized snapshot", value);
    Ok(())
}
