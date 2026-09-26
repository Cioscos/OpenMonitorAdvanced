//! MessagePack payload encoding/decoding and `u32`-LE-length framing.
//!
//! Decoding never lets `rmp-serde`/`serde` allocate from an attacker-declared
//! length: [`validate_message`] walks the raw MessagePack bytes first,
//! bounding nesting depth and array/map element counts, using only bounds
//! checks over the existing byte slice (no allocation proportional to a
//! declared length). Only after that structural check passes do we hand the
//! bytes to `rmp_serde::from_slice`.

use crate::message::Message;
use crate::{IpcError, MAX_FRAME_BYTES};

/// Maximum nesting depth (arrays/maps) accepted while validating raw
/// MessagePack bytes before decoding.
const MAX_DEPTH: usize = 64;

/// Maximum element count accepted for a single array or map header.
const MAX_ELEMENTS: usize = 100_000;

/// Maximum number of bytes `FrameDecoder` will ever buffer at once: one
/// frame's worth (length prefix + the largest allowed payload).
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
            validate_elements(bytes, pos, n.saturating_mul(2), depth)
        }
        0xdf => {
            let n = read_u32(bytes, pos)? as usize;
            validate_elements(bytes, pos, n.saturating_mul(2), depth)
        }
        // fixmap
        0x80..=0x8f => {
            let n = (marker & 0x0f) as usize;
            validate_elements(bytes, pos, n * 2, depth)
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

fn validate_elements(
    bytes: &[u8],
    pos: &mut usize,
    count: usize,
    depth: usize,
) -> Result<(), IpcError> {
    if count > MAX_ELEMENTS {
        return Err(IpcError::Decode(format!(
            "array/map declares {count} elements, exceeding the {MAX_ELEMENTS} limit"
        )));
    }
    for _ in 0..count {
        validate_value(bytes, pos, depth + 1)?;
    }
    Ok(())
}

/// Structurally validates `bytes` as exactly one MessagePack value, bounding
/// depth and element counts before any `serde`-driven allocation happens.
/// Returns an error if there are leftover bytes after the value.
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
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Buffers `bytes`. Rejects the push if it would grow the internal
    /// buffer past `MAX_FRAME_BYTES + 4` (one frame's worth) — the buffer
    /// never grows unboundedly no matter how the reader feeds it.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), IpcError> {
        let new_len = self.buf.len().saturating_add(bytes.len());
        if new_len > MAX_BUFFERED_BYTES {
            return Err(IpcError::FrameTooLarge(
                new_len.min(u32::MAX as usize) as u32
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

    fn encode_fixstr(s: &str) -> Vec<u8> {
        let mut out = vec![0xa0 | s.len() as u8];
        out.extend_from_slice(s.as_bytes());
        out
    }
}
