//! MessagePack payload encoding/decoding and `u32`-LE-length framing.
//!
//! Decoding never lets `rmp-serde`/`serde` allocate from an attacker-declared
//! length: [`validate_message`] walks the raw MessagePack bytes first,
//! bounding nesting depth and array/map element counts, using only bounds
//! checks over the existing byte slice (no allocation proportional to a
//! declared length), and rejecting a map with a duplicate string key. Only
//! after that structural check passes do we hand the bytes to
//! `rmp_serde::from_slice`.

use std::collections::HashSet;

use crate::message::Message;
use crate::{IpcError, MAX_FRAME_BYTES};

/// Maximum nesting depth (arrays/maps) accepted while validating raw
/// MessagePack bytes before decoding.
const MAX_DEPTH: usize = 64;

/// Maximum element count accepted for a single array or map header
/// (ruling R9): for an array, the number of items; for a map, the number of
/// key/value **entries** (not doubled for the two values per entry). Both
/// sides of the wire (this crate and the .NET codec, Task 3) must apply the
/// same rule — see `protocol/fixtures/README.md`.
const MAX_ELEMENTS: usize = 100_000;

/// Maximum number of bytes `FrameDecoder` will ever buffer *before* a call
/// to [`FrameDecoder::push`] — see that method's contract (ruling R8).
const MAX_BUFFERED_BYTES: usize = MAX_FRAME_BYTES + 4;

fn eof() -> IpcError {
    IpcError::Decode("unexpected end of message".to_owned())
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, IpcError> {
    let b = *bytes.get(*pos).ok_or_else(eof)?;
    *pos += 1;
    Ok(b)
}

fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, IpcError> {
    let end = pos.checked_add(2).ok_or_else(eof)?;
    let slice = bytes.get(*pos..end).ok_or_else(eof)?;
    let v = u16::from_be_bytes(slice.try_into().expect("2-byte slice"));
    *pos = end;
    Ok(v)
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, IpcError> {
    let end = pos.checked_add(4).ok_or_else(eof)?;
    let slice = bytes.get(*pos..end).ok_or_else(eof)?;
    let v = u32::from_be_bytes(slice.try_into().expect("4-byte slice"));
    *pos = end;
    Ok(v)
}

fn skip(bytes: &[u8], pos: &mut usize, n: usize) -> Result<(), IpcError> {
    let end = pos.checked_add(n).ok_or_else(eof)?;
    if end > bytes.len() {
        return Err(eof());
    }
    *pos = end;
    Ok(())
}

/// Validates one MessagePack value starting at `*pos`, advancing `*pos`
/// past it. Never allocates memory proportional to a declared length: array
/// and map elements are visited by recursive bounds-checked scanning of the
/// existing slice, and strings/bin/ext payloads are only skipped over.
fn validate_value(bytes: &[u8], pos: &mut usize, depth: usize) -> Result<(), IpcError> {
    if depth > MAX_DEPTH {
        return Err(IpcError::Decode(
            "message nesting exceeds the maximum depth".to_owned(),
        ));
    }
    let marker = read_u8(bytes, pos)?;
    match marker {
        // positive fixint, negative fixint, nil, bool
        0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => Ok(()),
        0xc1 => Err(IpcError::Decode(
            "reserved MessagePack marker 0xc1".to_owned(),
        )),
        // bin8 / bin16 / bin32
        0xc4 => {
            let n = read_u8(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        0xc5 => {
            let n = read_u16(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        0xc6 => {
            let n = read_u32(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        // ext8 / ext16 / ext32: length, then a 1-byte type tag, then data.
        0xc7 => {
            let n = read_u8(bytes, pos)? as usize;
            skip(bytes, pos, 1)?;
            skip(bytes, pos, n)
        }
        0xc8 => {
            let n = read_u16(bytes, pos)? as usize;
            skip(bytes, pos, 1)?;
            skip(bytes, pos, n)
        }
        0xc9 => {
            let n = read_u32(bytes, pos)? as usize;
            skip(bytes, pos, 1)?;
            skip(bytes, pos, n)
        }
        0xca => skip(bytes, pos, 4),  // f32
        0xcb => skip(bytes, pos, 8),  // f64
        0xcc => skip(bytes, pos, 1),  // u8
        0xcd => skip(bytes, pos, 2),  // u16
        0xce => skip(bytes, pos, 4),  // u32
        0xcf => skip(bytes, pos, 8),  // u64
        0xd0 => skip(bytes, pos, 1),  // i8
        0xd1 => skip(bytes, pos, 2),  // i16
        0xd2 => skip(bytes, pos, 4),  // i32
        0xd3 => skip(bytes, pos, 8),  // i64
        0xd4 => skip(bytes, pos, 2),  // fixext1 (1 type byte + 1 data byte)
        0xd5 => skip(bytes, pos, 3),  // fixext2
        0xd6 => skip(bytes, pos, 5),  // fixext4
        0xd7 => skip(bytes, pos, 9),  // fixext8
        0xd8 => skip(bytes, pos, 17), // fixext16
        0xd9 => {
            let n = read_u8(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        0xda => {
            let n = read_u16(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        0xdb => {
            let n = read_u32(bytes, pos)? as usize;
            skip(bytes, pos, n)
        }
        0xdc => {
            let n = read_u16(bytes, pos)? as usize;
            validate_elements(bytes, pos, n, depth)
        }
        0xdd => {
            let n = read_u32(bytes, pos)? as usize;
            validate_elements(bytes, pos, n, depth)
        }
        0xde => {
            let n = read_u16(bytes, pos)? as usize;
            validate_map(bytes, pos, n, depth)
        }
        0xdf => {
            let n = read_u32(bytes, pos)? as usize;
            validate_map(bytes, pos, n, depth)
        }
        // fixmap
        0x80..=0x8f => {
            let n = (marker & 0x0f) as usize;
            validate_map(bytes, pos, n, depth)
        }
        // fixarray
        0x90..=0x9f => {
            let n = (marker & 0x0f) as usize;
            validate_elements(bytes, pos, n, depth)
        }
        // fixstr
        0xa0..=0xbf => {
            let n = (marker & 0x1f) as usize;
            skip(bytes, pos, n)
        }
    }
}

/// Validates `count` array elements (ruling R9: an array's limit is its item
/// count, checked directly against [`MAX_ELEMENTS`]).
fn validate_elements(
    bytes: &[u8],
    pos: &mut usize,
    count: usize,
    depth: usize,
) -> Result<(), IpcError> {
    if count > MAX_ELEMENTS {
        return Err(IpcError::Decode(format!(
            "array declares {count} elements, exceeding the {MAX_ELEMENTS} limit"
        )));
    }
    for _ in 0..count {
        validate_value(bytes, pos, depth + 1)?;
    }
    Ok(())
}

/// Validates `count` map entries (ruling R9: a map's limit is its
/// key/value-pair count, not the number of values scanned — so a map
/// declaring exactly [`MAX_ELEMENTS`] pairs passes the count check, even
/// though `2 * MAX_ELEMENTS` individual values are then visited).
///
/// Also rejects a map that repeats the same string key twice. Two keys are
/// compared by their **decoded UTF-8 payload bytes**, not their raw encoded
/// bytes: a hostile message could otherwise encode the same key string
/// with two different marker families (e.g. `"type"` as a 5-byte fixstr and
/// again as a 6-byte str8) and slip past a check that only compared the raw
/// bytes, since those would differ even though the decoded key is
/// identical. A `HashSet` keyed by each key's decoded payload slice (never
/// allocated larger than `count`, which is itself bounded by
/// `MAX_ELEMENTS`) is used instead of pairwise comparison, to stay well
/// clear of `O(n^2)` behaviour even at the limit. Non-string keys are not
/// deduplicated: the wire protocol only ever uses string keys for struct
/// fields and for `WireDevice.properties`.
fn validate_map(bytes: &[u8], pos: &mut usize, count: usize, depth: usize) -> Result<(), IpcError> {
    if count > MAX_ELEMENTS {
        return Err(IpcError::Decode(format!(
            "map declares {count} entries, exceeding the {MAX_ELEMENTS} limit"
        )));
    }
    let mut seen_keys: HashSet<&[u8]> = HashSet::new();
    for _ in 0..count {
        let key_start = *pos;
        validate_value(bytes, pos, depth + 1)?;
        let key_bytes = &bytes[key_start..*pos];
        if let Some(payload) = string_key_payload(key_bytes) {
            if !seen_keys.insert(payload) {
                return Err(IpcError::Decode("map has a duplicate key".to_owned()));
            }
        }
        validate_value(bytes, pos, depth + 1)?;
    }
    Ok(())
}

/// If `key_bytes` (a fully-scanned MessagePack value) is a string (fixstr,
/// str8, str16 or str32), returns its decoded payload bytes — the raw UTF-8
/// content, with the marker and length header stripped off, so two keys
/// encoding the same string with different marker families compare equal.
fn string_key_payload(key_bytes: &[u8]) -> Option<&[u8]> {
    match *key_bytes.first()? {
        marker @ 0xa0..=0xbf => {
            let len = (marker & 0x1f) as usize;
            key_bytes.get(1..1 + len)
        }
        0xd9 => {
            let len = *key_bytes.get(1)? as usize;
            key_bytes.get(2..2 + len)
        }
        0xda => {
            let len = u16::from_be_bytes(key_bytes.get(1..3)?.try_into().ok()?) as usize;
            key_bytes.get(3..3 + len)
        }
        0xdb => {
            let len = u32::from_be_bytes(key_bytes.get(1..5)?.try_into().ok()?) as usize;
            key_bytes.get(5..5 + len)
        }
        _ => None,
    }
}

/// Structurally validates `bytes` as exactly one MessagePack value, bounding
/// depth and element counts and rejecting duplicate map keys before any
/// `serde`-driven allocation happens. Returns an error if there are leftover
/// bytes after the value.
fn validate_message(bytes: &[u8]) -> Result<(), IpcError> {
    if bytes.is_empty() {
        return Err(IpcError::Decode("empty payload".to_owned()));
    }
    let mut pos = 0usize;
    validate_value(bytes, &mut pos, 0)?;
    if pos != bytes.len() {
        return Err(IpcError::Decode(
            "trailing bytes after the MessagePack message".to_owned(),
        ));
    }
    Ok(())
}

/// Encodes a [`Message`] as a MessagePack payload (`rmp_serde::to_vec_named`:
/// structs as maps, keyed by field name, in declared order).
pub fn encode_payload(msg: &Message) -> Result<Vec<u8>, IpcError> {
    rmp_serde::to_vec_named(msg).map_err(|e| IpcError::Encode(e.to_string()))
}

/// Encodes a [`Message`] as a full frame: a little-endian `u32` payload
/// length, followed by the MessagePack payload. Refuses payloads over
/// [`MAX_FRAME_BYTES`].
pub fn encode_frame(msg: &Message) -> Result<Vec<u8>, IpcError> {
    let payload = encode_payload(msg)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(payload.len() as u32));
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decodes a MessagePack payload (no length prefix) into a [`Message`].
///
/// Validates the raw bytes' structure (depth, element counts) before
/// deserializing, rejects empty payloads and trailing bytes after the
/// message, and replaces any non-finite `Snapshot` value with `None`
/// defensively (values must never be NaN/infinite on the wire, but a
/// malformed or hostile sender must not be able to smuggle one through).
pub fn decode_payload(bytes: &[u8]) -> Result<Message, IpcError> {
    validate_message(bytes)?;
    let mut msg: Message =
        rmp_serde::from_slice(bytes).map_err(|e| IpcError::Decode(e.to_string()))?;
    if let Message::Snapshot(snapshot) = &mut msg {
        for value in snapshot.values.iter_mut() {
            if let Some(v) = *value {
                if !v.is_finite() {
                    *value = None;
                }
            }
        }
    }
    Ok(msg)
}

/// Incremental decoder for the `u32`-LE-length-prefixed frame stream read
/// from the sensor pipe. Fed by an overlapped reader (Task 8): bytes arrive
/// in arbitrary chunks, and `push`/`next_message` must never block or
/// allocate proportionally to an attacker-declared length.
///
/// # Contract (ruling R8)
///
/// The caller must call [`next_message`](Self::next_message) repeatedly
/// after every [`push`](Self::push), until it returns `Ok(None)`, before
/// pushing more bytes. Under that contract the buffer can briefly hold more
/// than one frame's worth of bytes — for example when a single read returns
/// the tail of one frame fused with the head of the next — but it always
/// starts each `push` at or below `MAX_FRAME_BYTES + 4` buffered bytes,
/// because the previous `push`'s frames were drained down to at most one
/// incomplete frame first. `push` only rejects a call that starts with
/// *more* than `MAX_FRAME_BYTES + 4` bytes already buffered, which can only
/// happen if the caller violated the contract (kept pushing without
/// draining). The incoming chunk itself is expected to be bounded by the
/// caller's own read buffer size (Task 8's reader uses at most 64 KiB per
/// read); `push` does not re-validate that.
///
/// # Errors are fatal
///
/// Any `Err` returned by `push`, `next_message` or `finish` means the byte
/// stream is no longer trustworthy (a hostile/corrupt frame, an oversized
/// declared length, or a contract violation). The caller must drop the
/// connection; a `FrameDecoder` makes no attempt to resynchronize with the
/// stream after an error, and continuing to feed it more bytes is not
/// supported.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Buffers `bytes`. See the struct-level contract (ruling R8): this
    /// only rejects the push if *more* than `MAX_FRAME_BYTES + 4` bytes were
    /// already buffered before this call — a contract violation, not a
    /// declared frame length, so it never carries a length in its error.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), IpcError> {
        if self.buf.len() > MAX_BUFFERED_BYTES {
            return Err(IpcError::Decode(
                "push called with more than one frame's worth already buffered; \
                 drain next_message() to Ok(None) after every push"
                    .to_owned(),
            ));
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    /// Returns the next complete message, if the buffer holds one.
    /// `Ok(None)` means "not enough bytes yet, keep pushing" — a clean
    /// state distinct from an error. A declared frame length over
    /// [`MAX_FRAME_BYTES`] is rejected immediately, before waiting for (or
    /// allocating for) the rest of the oversized frame.
    pub fn next_message(&mut self) -> Result<Option<Message>, IpcError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(self.buf[0..4].try_into().expect("4-byte slice"));
        if len as usize > MAX_FRAME_BYTES {
            self.buf.clear();
            self.buf.shrink_to_fit();
            return Err(IpcError::FrameTooLarge(len));
        }
        let total = 4 + len as usize;
        if self.buf.len() < total {
            return Ok(None);
        }
        let message = decode_payload(&self.buf[4..total])?;
        self.buf.drain(0..total);
        Ok(Some(message))
    }

    /// Call once the underlying stream has hit a clean EOF (no more bytes
    /// will ever arrive). `Ok(())` means the decoder was idle (no partial
    /// frame in flight); `Err` means a header or body was left truncated —
    /// a distinct condition from a clean EOF with nothing pending.
    pub fn finish(&self) -> Result<(), IpcError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(IpcError::Decode(
                "stream ended with a truncated frame".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Subscribe, WireSnapshot};

    fn subscribe(interval_ms: u32) -> Message {
        Message::Subscribe(Subscribe { interval_ms })
    }

    #[test]
    fn two_frames_in_one_push_decode_in_order() {
        let a = encode_frame(&subscribe(500)).unwrap();
        let b = encode_frame(&subscribe(1000)).unwrap();
        let mut combined = a;
        combined.extend_from_slice(&b);

        let mut decoder = FrameDecoder::new();
        decoder.push(&combined).unwrap();

        assert_eq!(decoder.next_message().unwrap(), Some(subscribe(500)));
        assert_eq!(decoder.next_message().unwrap(), Some(subscribe(1000)));
        assert_eq!(decoder.next_message().unwrap(), None);
    }

    #[test]
    fn truncated_frame_waits_for_more_bytes() {
        let frame = encode_frame(&subscribe(500)).unwrap();
        let (head, tail) = frame.split_at(frame.len() - 2);

        let mut decoder = FrameDecoder::new();
        decoder.push(head).unwrap();
        assert_eq!(decoder.next_message().unwrap(), None);

        decoder.push(tail).unwrap();
        assert_eq!(decoder.next_message().unwrap(), Some(subscribe(500)));
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocating() {
        let declared_len: u32 = MAX_FRAME_BYTES as u32 + 1;
        let mut input = declared_len.to_le_bytes().to_vec();
        input.extend_from_slice(&[0u8; 4]);

        let mut decoder = FrameDecoder::new();
        decoder.push(&input).unwrap();

        let err = decoder.next_message().unwrap_err();
        assert!(matches!(err, IpcError::FrameTooLarge(4_194_305)));
        assert!(
            decoder.buf.capacity() < 1024,
            "buffer capacity grew to {} bytes, should stay under 1 KiB",
            decoder.buf.capacity()
        );
    }

    #[test]
    fn push_accepts_a_frame_tail_fused_with_the_next_frames_head() {
        // Build a payload of exactly MAX_FRAME_BYTES: measure the envelope's
        // fixed overhead with a placeholder length (long enough to already
        // need a str32 header, same as the final message), then size the
        // filler so the total lands exactly on the limit.
        let overhead_probe_len = MAX_FRAME_BYTES - 100;
        let overhead_probe = encode_payload(&Message::Error(crate::message::WireError {
            code: "bad_request".to_owned(),
            message: "x".repeat(overhead_probe_len),
        }))
        .unwrap();
        let overhead = overhead_probe.len() - overhead_probe_len;
        let big_msg = Message::Error(crate::message::WireError {
            code: "bad_request".to_owned(),
            message: "x".repeat(MAX_FRAME_BYTES - overhead),
        });
        let big_frame = encode_frame(&big_msg).unwrap();
        // The big frame fills the buffer to exactly its cap.
        assert_eq!(big_frame.len(), MAX_FRAME_BYTES + 4);

        let next_msg = Message::Error(crate::message::WireError {
            code: "bad_request".to_owned(),
            message: "y".repeat(40),
        });
        let next_frame = encode_frame(&next_msg).unwrap();

        let split = big_frame.len() - 5;
        let (head, tail) = big_frame.split_at(split);

        let mut decoder = FrameDecoder::new();
        decoder.push(head).unwrap();
        assert_eq!(decoder.next_message().unwrap(), None);
        assert!(decoder.buf.len() <= MAX_BUFFERED_BYTES);

        // Fuse the big frame's tail with the head of the next frame, as one
        // read from the pipe legitimately could. The buffer briefly holds
        // more than MAX_BUFFERED_BYTES once this is appended -- legal per
        // ruling R8, since the buffer was drained to Ok(None) beforehand.
        let next_head_len = 40.min(next_frame.len() - 1);
        let mut fused = tail.to_vec();
        fused.extend_from_slice(&next_frame[..next_head_len]);
        let buffered_before_fuse = decoder.buf.len();
        decoder.push(&fused).unwrap();
        assert!(
            decoder.buf.len() > MAX_BUFFERED_BYTES,
            "test setup should have pushed the buffer transiently over the cap \
             ({buffered_before_fuse} + {} = {}, cap {MAX_BUFFERED_BYTES})",
            fused.len(),
            decoder.buf.len()
        );

        assert_eq!(decoder.next_message().unwrap(), Some(big_msg));
        assert_eq!(decoder.buf.len(), next_head_len);
        assert_eq!(decoder.next_message().unwrap(), None);

        decoder.push(&next_frame[next_head_len..]).unwrap();
        assert_eq!(decoder.next_message().unwrap(), Some(next_msg));
    }

    #[test]
    fn buffer_stays_bounded_under_fragmented_pushes() {
        let msg = Message::Error(crate::message::WireError {
            code: "bad_request".to_owned(),
            message: "z".repeat(MAX_FRAME_BYTES - 1000),
        });
        let frame = encode_frame(&msg).unwrap();

        let mut decoder = FrameDecoder::new();
        let mut decoded = None;
        for chunk in frame.chunks(4096) {
            decoder.push(chunk).unwrap();
            assert!(
                decoder.buf.len() <= MAX_BUFFERED_BYTES,
                "buffer grew to {} bytes while feeding fragmented input",
                decoder.buf.len()
            );
            if let Some(m) = decoder.next_message().unwrap() {
                decoded = Some(m);
            }
        }
        assert_eq!(decoded, Some(msg));
    }

    #[test]
    fn zero_length_frame_is_a_decode_error() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&0u32.to_le_bytes()).unwrap();
        let err = decoder.next_message().unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn encode_frame_refuses_a_payload_over_the_limit() {
        let huge = "x".repeat(MAX_FRAME_BYTES + 1);
        let msg = Message::Error(crate::message::WireError {
            code: "bad_request".to_owned(),
            message: huge,
        });

        let err = encode_frame(&msg).unwrap_err();
        assert!(matches!(err, IpcError::FrameTooLarge(_)));
    }

    #[test]
    fn unknown_message_type_is_a_decode_error() {
        // {"type": "ping", "body": {}}
        let mut bytes = vec![0x82]; // fixmap, 2 entries
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("ping"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x80); // empty fixmap

        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn extra_fields_are_ignored() {
        // {"type": "subscribe", "body": {"interval_ms": 500, "extra": 1}}
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("subscribe"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x82); // body fixmap, 2 entries
        bytes.extend_from_slice(&encode_fixstr("interval_ms"));
        bytes.push(0xcd); // uint16
        bytes.extend_from_slice(&500u16.to_be_bytes());
        bytes.extend_from_slice(&encode_fixstr("extra"));
        bytes.push(0x01); // fixint 1

        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded, subscribe(500));
    }

    #[test]
    fn non_finite_values_decode_as_missing() {
        // {"type": "snapshot", "body": {"seq": 1, "timestamp_ms": 0, "values": [NaN, +inf]}}
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("snapshot"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x83); // body fixmap, 3 entries
        bytes.extend_from_slice(&encode_fixstr("seq"));
        bytes.push(0x01);
        bytes.extend_from_slice(&encode_fixstr("timestamp_ms"));
        bytes.push(0x00);
        bytes.extend_from_slice(&encode_fixstr("values"));
        bytes.push(0x92); // fixarray, 2 elements
        bytes.push(0xcb); // float64 NaN
        bytes.extend_from_slice(&f64::NAN.to_be_bytes());
        bytes.push(0xcb); // float64 +inf
        bytes.extend_from_slice(&f64::INFINITY.to_be_bytes());

        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(
            decoded,
            Message::Snapshot(WireSnapshot {
                seq: 1,
                timestamp_ms: 0,
                values: vec![None, None],
            })
        );
    }

    #[test]
    fn partial_frame_at_eof_is_an_error() {
        let frame = encode_frame(&subscribe(500)).unwrap();
        let (head, _tail) = frame.split_at(frame.len() - 2);

        let mut decoder = FrameDecoder::new();
        decoder.push(head).unwrap();
        assert_eq!(decoder.next_message().unwrap(), None);

        let err = decoder.finish().unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn clean_eof_is_ok() {
        let frame = encode_frame(&subscribe(500)).unwrap();
        let mut decoder = FrameDecoder::new();
        decoder.push(&frame).unwrap();
        assert_eq!(decoder.next_message().unwrap(), Some(subscribe(500)));
        assert!(decoder.finish().is_ok());

        // Never having pushed anything is also a clean EOF.
        assert!(FrameDecoder::new().finish().is_ok());
    }

    #[test]
    fn empty_payload_is_rejected() {
        let err = decode_payload(&[]).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = encode_payload(&subscribe(500)).unwrap();
        bytes.push(0xc0); // an extra nil byte tacked on after a valid message
        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn reserved_marker_c1_is_rejected() {
        let err = decode_payload(&[0xc1]).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn missing_required_field_is_rejected() {
        // {"type": "subscribe", "body": {}}
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("subscribe"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x80);

        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn duplicate_field_in_body_is_rejected() {
        // {"type": "subscribe", "body": {"interval_ms": 500, "interval_ms": 999}}
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("subscribe"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x82);
        bytes.extend_from_slice(&encode_fixstr("interval_ms"));
        bytes.push(0xcd);
        bytes.extend_from_slice(&500u16.to_be_bytes());
        bytes.extend_from_slice(&encode_fixstr("interval_ms"));
        bytes.push(0xcd);
        bytes.extend_from_slice(&999u16.to_be_bytes());

        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn duplicate_envelope_type_key_is_rejected() {
        // {"type": "subscribe", "type": "hello", "body": {"interval_ms": 500}}
        let mut bytes = vec![0x83];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("subscribe"));
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("hello"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x81);
        bytes.extend_from_slice(&encode_fixstr("interval_ms"));
        bytes.push(0xcd);
        bytes.extend_from_slice(&500u16.to_be_bytes());

        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn duplicate_key_with_different_str_markers_is_rejected() {
        // A hostile message repeats the same decoded key inside a
        // `WireDevice.properties` map (a plain `BTreeMap<String, String>`,
        // not a fixed-shape struct) but switches MessagePack marker family
        // between the two occurrences. `BTreeMap`'s own `Deserialize` does
        // NOT reject a duplicate key by itself (the second value simply
        // overwrites the first), so this is a case that is caught only by
        // the raw scanner's duplicate-key check comparing *decoded* string
        // content -- a check that compared raw encoded bytes would miss it,
        // since a fixstr encoding and a str8 encoding of "firmware" differ
        // byte-for-byte even though they decode to the same key.
        assert!(matches!(
            device_with_duplicate_property_key(&encode_str8).unwrap_err(),
            IpcError::Decode(_)
        ));
        assert!(matches!(
            device_with_duplicate_property_key(&encode_str16).unwrap_err(),
            IpcError::Decode(_)
        ));
    }

    /// Builds `{"type":"schema","body":{"devices":[{...,"properties":{"firmware"(fixstr):"A","firmware"(<second_encoding>):"B"}, ...}],"sensors":[]}}`
    /// and decodes it, so the caller can assert on the duplicate-key
    /// behaviour for a generic (non-struct) map.
    fn device_with_duplicate_property_key(
        second_encoding: &dyn Fn(&str) -> Vec<u8>,
    ) -> Result<Message, IpcError> {
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("schema"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x82); // body: devices, sensors
        bytes.extend_from_slice(&encode_fixstr("devices"));
        bytes.push(0x91); // 1 device
        bytes.push(0x86); // device map: 6 entries
        bytes.extend_from_slice(&encode_fixstr("id"));
        bytes.extend_from_slice(&encode_fixstr("d"));
        bytes.extend_from_slice(&encode_fixstr("kind"));
        bytes.extend_from_slice(&encode_fixstr("cpu"));
        bytes.extend_from_slice(&encode_fixstr("name"));
        bytes.extend_from_slice(&encode_fixstr("n"));
        bytes.extend_from_slice(&encode_fixstr("vendor"));
        bytes.push(0xc0); // nil
        bytes.extend_from_slice(&encode_fixstr("properties"));
        bytes.push(0x82); // properties map: 2 entries, duplicate key
        bytes.extend_from_slice(&encode_fixstr("firmware")); // fixstr
        bytes.extend_from_slice(&encode_fixstr("A"));
        bytes.extend_from_slice(&second_encoding("firmware")); // str8 or str16
        bytes.extend_from_slice(&encode_fixstr("B"));
        bytes.extend_from_slice(&encode_fixstr("hint"));
        bytes.push(0xc0); // nil
        bytes.extend_from_slice(&encode_fixstr("sensors"));
        bytes.push(0x90); // []

        decode_payload(&bytes)
    }

    #[test]
    fn array32_declaring_u32_max_elements_is_rejected() {
        let bytes = [0xdd, 0xff, 0xff, 0xff, 0xff];
        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn map32_over_limit_is_rejected() {
        let count: u32 = 100_001;
        let mut bytes = vec![0xdf];
        bytes.extend_from_slice(&count.to_be_bytes());
        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn map_with_exactly_100000_entries_header_passes_the_count_check() {
        let count: u32 = 100_000;
        let mut bytes = vec![0xdf];
        bytes.extend_from_slice(&count.to_be_bytes());
        // No entries follow: the count check itself must accept exactly
        // MAX_ELEMENTS, so the failure that does occur must come from
        // running out of bytes while reading the first entry, not from the
        // element-count limit.
        let err = decode_payload(&bytes).unwrap_err();
        match err {
            IpcError::Decode(msg) => assert!(
                !msg.contains("exceeding"),
                "expected an EOF-style error, not a limit violation: {msg}"
            ),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn str32_declaring_4gb_is_rejected_without_allocating() {
        let bytes = [0xdb, 0xff, 0xff, 0xff, 0xff];
        let err = decode_payload(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn nesting_deeper_than_64_is_rejected() {
        let mut bytes = vec![0x91; 65]; // 65 nested one-element fixarrays
        bytes.push(0xc0); // innermost nil
        let err = validate_message(&bytes).unwrap_err();
        assert!(matches!(err, IpcError::Decode(_)));
    }

    #[test]
    fn nesting_of_exactly_64_is_accepted_by_the_scanner() {
        let mut bytes = vec![0x91; 64];
        bytes.push(0xc0);
        assert!(validate_message(&bytes).is_ok());
    }

    #[test]
    fn omitted_optional_key_decodes_as_none() {
        // WireDevice with `vendor` entirely absent (not `nil`) -- ruling R10:
        // the decoder tolerates this even though an encoder must never do it.
        use crate::message::{WireDevice, WireSchema};
        use std::collections::BTreeMap;

        // {"type": "schema", "body": {"devices": [{"id":"d","kind":"cpu","name":"n","properties":{},"hint":nil}], "sensors": []}}
        let mut bytes = vec![0x82];
        bytes.extend_from_slice(&encode_fixstr("type"));
        bytes.extend_from_slice(&encode_fixstr("schema"));
        bytes.extend_from_slice(&encode_fixstr("body"));
        bytes.push(0x82); // body: devices, sensors
        bytes.extend_from_slice(&encode_fixstr("devices"));
        bytes.push(0x91); // 1 device
        bytes.push(0x85); // device map: 5 entries (vendor omitted on purpose)
        bytes.extend_from_slice(&encode_fixstr("id"));
        bytes.extend_from_slice(&encode_fixstr("d"));
        bytes.extend_from_slice(&encode_fixstr("kind"));
        bytes.extend_from_slice(&encode_fixstr("cpu"));
        bytes.extend_from_slice(&encode_fixstr("name"));
        bytes.extend_from_slice(&encode_fixstr("n"));
        bytes.extend_from_slice(&encode_fixstr("properties"));
        bytes.push(0x80); // {}
        bytes.extend_from_slice(&encode_fixstr("hint"));
        bytes.push(0xc0); // nil
        bytes.extend_from_slice(&encode_fixstr("sensors"));
        bytes.push(0x90); // []

        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(
            decoded,
            Message::Schema(WireSchema {
                devices: vec![WireDevice {
                    id: "d".to_owned(),
                    kind: "cpu".to_owned(),
                    name: "n".to_owned(),
                    vendor: None,
                    properties: BTreeMap::new(),
                    hint: None,
                }],
                sensors: vec![],
            })
        );
    }

    fn encode_fixstr(s: &str) -> Vec<u8> {
        let mut out = vec![0xa0 | s.len() as u8];
        out.extend_from_slice(s.as_bytes());
        out
    }

    fn encode_str8(s: &str) -> Vec<u8> {
        let mut out = vec![0xd9, s.len() as u8];
        out.extend_from_slice(s.as_bytes());
        out
    }

    fn encode_str16(s: &str) -> Vec<u8> {
        let mut out = vec![0xda];
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
        out
    }
}
