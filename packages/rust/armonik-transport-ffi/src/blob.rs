use armonik_transport::grpc::{Metadata, MetadataValue, BINARY_SUFFIX};
use bytes::Bytes;

pub(crate) type Pairs<'a> = Vec<(&'a [u8], &'a [u8])>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlobError {
    Truncated,
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Pairs<'_>, BlobError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut cursor = bytes;
    let count = read_u32(&mut cursor)? as usize;

    if count.saturating_mul(8) > cursor.len() {
        return Err(BlobError::Truncated);
    }

    let mut pairs = Pairs::with_capacity(count);
    for _ in 0..count {
        let key = read_chunk(&mut cursor)?;
        let value = read_chunk(&mut cursor)?;
        pairs.push((key, value));
    }
    Ok(pairs)
}

pub(crate) fn encode<'a>(pairs: impl ExactSizeIterator<Item = (&'a [u8], &'a [u8])>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(pairs.len() as u32).to_ne_bytes());
    for (key, value) in pairs {
        out.extend_from_slice(&(key.len() as u32).to_ne_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&(value.len() as u32).to_ne_bytes());
        out.extend_from_slice(value);
    }
    out
}

pub(crate) fn decode_metadata(bytes: &[u8]) -> Option<Metadata> {
    let mut metadata = Metadata::new();
    for (key, value) in decode(bytes).ok()? {
        let key = std::str::from_utf8(key).ok()?;
        let entry = if key.ends_with(BINARY_SUFFIX) {
            MetadataValue::Binary(Bytes::copy_from_slice(value))
        } else {
            MetadataValue::Ascii(std::str::from_utf8(value).ok()?.to_owned())
        };
        metadata.append(key, entry).ok()?;
    }
    Some(metadata)
}

pub(crate) fn encode_metadata(metadata: &Metadata) -> Vec<u8> {
    encode(metadata.iter().map(|(key, value)| {
        let value: &[u8] = match value {
            MetadataValue::Ascii(text) => text.as_bytes(),
            MetadataValue::Binary(bytes) => bytes,
        };
        (key.as_bytes(), value)
    }))
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32, BlobError> {
    if cursor.len() < 4 {
        return Err(BlobError::Truncated);
    }
    let (head, rest) = cursor.split_at(4);
    *cursor = rest;
    Ok(u32::from_ne_bytes([head[0], head[1], head[2], head[3]]))
}

fn read_chunk<'a>(cursor: &mut &'a [u8]) -> Result<&'a [u8], BlobError> {
    let len = read_u32(cursor)? as usize;
    if cursor.len() < len {
        return Err(BlobError::Truncated);
    }
    let (chunk, rest) = cursor.split_at(len);
    *cursor = rest;
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(pairs: &[(&[u8], &[u8])]) -> Vec<u8> {
        encode(pairs.iter().copied())
    }

    #[test]
    fn pairs_come_back_in_the_order_they_went_in() {
        let written = blob(&[(b"a", b"1"), (b"b", b"2"), (b"a", b"3")]);
        assert_eq!(
            decode(&written).expect("well-formed"),
            vec![
                (&b"a"[..], &b"1"[..]),
                (&b"b"[..], &b"2"[..]),
                (&b"a"[..], &b"3"[..])
            ]
        );
    }

    #[test]
    fn nothing_at_all_reads_as_no_pairs() {
        assert_eq!(decode(&[]).expect("empty is legal"), Pairs::new());
        assert_eq!(decode(&blob(&[])).expect("a count of zero"), Pairs::new());
    }

    #[test]
    fn an_empty_key_or_value_is_a_pair_and_not_an_absence() {
        let written = blob(&[(b"", b""), (b"k", b"")]);
        assert_eq!(
            decode(&written).expect("well-formed"),
            vec![(&b""[..], &b""[..]), (&b"k"[..], &b""[..])]
        );
    }

    #[test]
    fn a_length_running_past_the_end_is_refused_rather_than_read() {
        let mut written = blob(&[(b"key", b"value")]);
        written.truncate(written.len() - 2);
        assert_eq!(decode(&written), Err(BlobError::Truncated));

        let mut absurd = u32::MAX.to_ne_bytes().to_vec();
        absurd.extend_from_slice(b"short");
        assert_eq!(decode(&absurd), Err(BlobError::Truncated));
    }

    #[test]
    fn metadata_survives_the_round_trip_with_its_binary_values_as_bytes() {
        let written = blob(&[(b"x-trace-bin", &[0, 1, 2, 0xff]), (b"x-plain", b"text")]);
        let metadata = decode_metadata(&written).expect("valid metadata");

        assert_eq!(
            metadata.get("x-trace-bin"),
            Some(&MetadataValue::Binary(Bytes::from_static(&[0, 1, 2, 0xff])))
        );
        assert_eq!(
            metadata.get("x-plain"),
            Some(&MetadataValue::Ascii("text".to_owned()))
        );
        assert_eq!(decode_metadata(&encode_metadata(&metadata)), Some(metadata));
    }

    #[test]
    fn an_entry_the_engine_will_not_carry_is_refused_rather_than_dropped() {
        assert_eq!(decode_metadata(&blob(&[(b"content-type", b"x")])), None);
        assert_eq!(decode_metadata(&blob(&[(b"x-plain", &[0xff])])), None);
    }
}
