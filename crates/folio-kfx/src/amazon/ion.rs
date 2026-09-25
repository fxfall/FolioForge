//! Bounded Amazon-Ion binary value reader used by the Amazon KFX importer.
//!
//! This module deliberately keeps Ion values format-native.  The KFX semantic
//! decoder is the only layer that may turn them into Folio Semantic IR.

use thiserror::Error;

pub const VERSION_MARKER: [u8; 4] = [0xe0, 0x01, 0x00, 0xea];
const MAX_DEPTH: usize = 128;
// Some large DRM-free Kindle books exceed one million Ion values in a single
// native fragment. Keep parsing bounded while allowing those real-world books.
const MAX_VALUES: usize = 2_000_000;
const MAX_VARUINT_BYTES: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum IonValue {
    Null,
    Nop,
    Bool(bool),
    Int(i64),
    Float(Vec<u8>),
    Decimal(Vec<u8>),
    Timestamp(Vec<u8>),
    Symbol(u64),
    String(String),
    Clob(Vec<u8>),
    Blob(Vec<u8>),
    List(Vec<IonValue>),
    SExp(Vec<IonValue>),
    Struct(Vec<(u64, IonValue)>),
    Annotation {
        symbols: Vec<u64>,
        value: Box<IonValue>,
    },
    Reserved {
        type_code: u8,
        bytes: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IonSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IonError {
    #[error("Ion data is truncated at byte {0}")]
    Truncated(usize),
    #[error("Ion length or offset overflows at byte {0}")]
    Overflow(usize),
    #[error("invalid Ion version marker at byte {0}")]
    InvalidVersionMarker(usize),
    #[error("unsupported Ion major version {major}.{minor} at byte {offset}")]
    UnsupportedVersion { major: u8, minor: u8, offset: usize },
    #[error("invalid Ion boolean descriptor at byte {0}")]
    InvalidBoolean(usize),
    #[error("invalid UTF-8 in Ion string at byte {0}")]
    InvalidUtf8(usize),
    #[error("Ion integer cannot be represented as a signed 64-bit value at byte {0}")]
    IntegerOutOfRange(usize),
    #[error("invalid Ion value type {type_code} at byte {offset}")]
    InvalidType { type_code: u8, offset: usize },
    #[error("Ion nesting exceeds the configured limit at byte {0}")]
    NestingLimit(usize),
    #[error("Ion value count exceeds the configured limit")]
    ValueCountLimit,
    #[error("Ion annotation is empty at byte {0}")]
    EmptyAnnotation(usize),
    #[error("Ion annotation contains trailing bytes at byte {0}")]
    AnnotationTrailingData(usize),
    #[error("Ion value contains trailing bytes at byte {0}")]
    TrailingData(usize),
}

/// Decode all values in an Ion datagram. Version markers may occur at the
/// beginning and between values. `max_values` is separately bounded so a
/// hostile input cannot create unbounded nested allocations.
#[allow(dead_code)]
pub fn decode_datagram(bytes: &[u8]) -> Result<Vec<IonValue>, IonError> {
    let mut reader = Reader {
        bytes,
        position: 0,
        values_read: 0,
    };
    let mut values = Vec::new();
    while reader.position < bytes.len() {
        if reader.has_version_marker() {
            reader.read_version_marker()?;
            continue;
        }
        if values.len() >= MAX_VALUES {
            return Err(IonError::ValueCountLimit);
        }
        let (value, _) = reader.read_value(bytes.len(), 0)?;
        if !matches!(value, IonValue::Nop) {
            values.push(value);
        }
    }
    Ok(values)
}

/// Decode one Ion value in a caller-provided byte range and return its span.
/// A leading Ion version marker is accepted but is not part of the span.
pub fn decode_one(bytes: &[u8], start: usize, end: usize) -> Result<(IonValue, IonSpan), IonError> {
    if start > end || end > bytes.len() {
        return Err(IonError::Truncated(start));
    }
    let mut reader = Reader {
        bytes,
        position: start,
        values_read: 0,
    };
    if reader.has_version_marker() {
        reader.read_version_marker()?;
    }
    let value_start = reader.position;
    let (value, value_end) = reader.read_value(end, 0)?;
    Ok((
        value,
        IonSpan {
            start: value_start,
            end: value_end,
        },
    ))
}

pub fn struct_get(value: &IonValue, field_id: u64) -> Option<&IonValue> {
    match value {
        IonValue::Struct(fields) => fields
            .iter()
            .find_map(|(id, value)| (*id == field_id).then_some(value)),
        IonValue::Annotation { value, .. } => struct_get(value, field_id),
        _ => None,
    }
}

pub fn as_int(value: &IonValue) -> Option<i64> {
    match value {
        IonValue::Int(value) => Some(*value),
        _ => None,
    }
}

pub fn as_string(value: &IonValue) -> Option<&str> {
    match value {
        IonValue::String(value) => Some(value),
        _ => None,
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    values_read: usize,
}

impl Reader<'_> {
    fn has_version_marker(&self) -> bool {
        self.bytes
            .get(self.position..self.position.saturating_add(VERSION_MARKER.len()))
            == Some(VERSION_MARKER.as_slice())
    }

    fn read_version_marker(&mut self) -> Result<(), IonError> {
        let marker_offset = self.position;
        let marker_end = marker_offset
            .checked_add(VERSION_MARKER.len())
            .ok_or(IonError::Overflow(marker_offset))?;
        let marker = self
            .bytes
            .get(marker_offset..marker_end)
            .ok_or(IonError::Truncated(marker_offset))?;
        if marker[0] != 0xe0 || marker[3] != 0xea {
            return Err(IonError::InvalidVersionMarker(marker_offset));
        }
        if marker[1] != 1 || marker[2] != 0 {
            return Err(IonError::UnsupportedVersion {
                major: marker[1],
                minor: marker[2],
                offset: marker_offset,
            });
        }
        self.position = marker_end;
        Ok(())
    }

    fn read_value(
        &mut self,
        enclosing_end: usize,
        depth: usize,
    ) -> Result<(IonValue, usize), IonError> {
        if depth > MAX_DEPTH {
            return Err(IonError::NestingLimit(self.position));
        }
        if self.values_read >= MAX_VALUES {
            return Err(IonError::ValueCountLimit);
        }
        self.values_read += 1;
        let offset = self.position;
        let descriptor = self.read_byte(enclosing_end)?;
        let type_code = descriptor >> 4;
        let length_code = descriptor & 0x0f;

        if type_code == 0 {
            if length_code == 0x0f {
                return Ok((IonValue::Null, self.position));
            }
            let length = self.read_value_length(length_code, enclosing_end)?;
            self.skip(length, enclosing_end)?;
            return Ok((IonValue::Nop, self.position));
        }

        if length_code == 0x0f {
            return Ok((IonValue::Null, self.position));
        }

        if type_code == 1 {
            return match length_code {
                0 => Ok((IonValue::Bool(false), self.position)),
                1 => Ok((IonValue::Bool(true), self.position)),
                _ => Err(IonError::InvalidBoolean(offset)),
            };
        }

        match type_code {
            11 | 12 => {
                let length = self.read_value_length(length_code, enclosing_end)?;
                let value_end = self.checked_end(length, enclosing_end)?;
                let mut items = Vec::new();
                while self.position < value_end {
                    if items.len() >= MAX_VALUES {
                        return Err(IonError::ValueCountLimit);
                    }
                    if self.has_version_marker() {
                        self.read_version_marker()?;
                        continue;
                    }
                    let (item, _) = self.read_value(value_end, depth + 1)?;
                    if !matches!(item, IonValue::Nop) {
                        items.push(item);
                    }
                }
                let value = if type_code == 11 {
                    IonValue::List(items)
                } else {
                    IonValue::SExp(items)
                };
                Ok((value, value_end))
            }
            13 => self.read_struct(length_code, enclosing_end, depth, offset),
            14 => self.read_annotation(length_code, enclosing_end, depth, offset),
            2..=10 | 15 => {
                let length = self.read_value_length(length_code, enclosing_end)?;
                let raw = self.read_bytes(length, enclosing_end)?;
                let value = match type_code {
                    2 => IonValue::Int(read_integer(raw, false, offset)?),
                    3 => IonValue::Int(read_integer(raw, true, offset)?),
                    4 => IonValue::Float(raw.to_vec()),
                    5 => IonValue::Decimal(raw.to_vec()),
                    6 => IonValue::Timestamp(raw.to_vec()),
                    7 => IonValue::Symbol(read_unsigned_integer(raw, offset)?),
                    8 => IonValue::String(
                        std::str::from_utf8(raw)
                            .map_err(|_| IonError::InvalidUtf8(offset))?
                            .to_owned(),
                    ),
                    9 => IonValue::Clob(raw.to_vec()),
                    10 => IonValue::Blob(raw.to_vec()),
                    15 => IonValue::Reserved {
                        type_code,
                        bytes: raw.to_vec(),
                    },
                    _ => return Err(IonError::InvalidType { type_code, offset }),
                };
                Ok((value, self.position))
            }
            _ => Err(IonError::InvalidType { type_code, offset }),
        }
    }

    fn read_struct(
        &mut self,
        length_code: u8,
        enclosing_end: usize,
        depth: usize,
        offset: usize,
    ) -> Result<(IonValue, usize), IonError> {
        let length = if length_code == 14 {
            self.read_varuint(enclosing_end)?
        } else {
            u64::from(length_code)
        };
        let length = usize::try_from(length).map_err(|_| IonError::Overflow(offset))?;
        let value_end = self.checked_end(length, enclosing_end)?;
        let mut fields = Vec::new();
        while self.position < value_end {
            if fields.len() >= MAX_VALUES {
                return Err(IonError::ValueCountLimit);
            }
            let field_id = self.read_varuint(value_end)?;
            if field_id == 0 {
                return Err(IonError::InvalidType {
                    type_code: 13,
                    offset: self.position,
                });
            }
            let (value, _) = self.read_value(value_end, depth + 1)?;
            if !matches!(value, IonValue::Nop) {
                fields.push((field_id, value));
            }
        }
        Ok((IonValue::Struct(fields), value_end))
    }

    fn read_annotation(
        &mut self,
        length_code: u8,
        enclosing_end: usize,
        depth: usize,
        offset: usize,
    ) -> Result<(IonValue, usize), IonError> {
        let length = self.read_value_length(length_code, enclosing_end)?;
        let value_end = self.checked_end(length, enclosing_end)?;
        let annotations_length = self.read_varuint(value_end)?;
        let annotations_length =
            usize::try_from(annotations_length).map_err(|_| IonError::Overflow(offset))?;
        let annotations_end = self.checked_end(annotations_length, value_end)?;
        let mut symbols = Vec::new();
        while self.position < annotations_end {
            if symbols.len() >= MAX_VALUES {
                return Err(IonError::ValueCountLimit);
            }
            symbols.push(self.read_varuint(annotations_end)?);
        }
        if symbols.is_empty() {
            return Err(IonError::EmptyAnnotation(offset));
        }
        if self.position >= value_end {
            return Err(IonError::Truncated(self.position));
        }
        let (value, _) = self.read_value(value_end, depth + 1)?;
        if self.position != value_end {
            return Err(IonError::AnnotationTrailingData(self.position));
        }
        Ok((
            IonValue::Annotation {
                symbols,
                value: Box::new(value),
            },
            value_end,
        ))
    }

    fn read_value_length(
        &mut self,
        length_code: u8,
        enclosing_end: usize,
    ) -> Result<usize, IonError> {
        let length = if length_code == 14 {
            self.read_varuint(enclosing_end)?
        } else {
            u64::from(length_code)
        };
        usize::try_from(length).map_err(|_| IonError::Overflow(self.position))
    }

    fn read_varuint(&mut self, enclosing_end: usize) -> Result<u64, IonError> {
        let start = self.position;
        let mut value = 0u64;
        for _ in 0..MAX_VARUINT_BYTES {
            let byte = self.read_byte(enclosing_end)?;
            value = value
                .checked_mul(128)
                .and_then(|value| value.checked_add(u64::from(byte & 0x7f)))
                .ok_or(IonError::Overflow(start))?;
            if byte & 0x80 != 0 {
                return Ok(value);
            }
        }
        Err(IonError::Overflow(start))
    }

    fn read_byte(&mut self, enclosing_end: usize) -> Result<u8, IonError> {
        if self.position >= enclosing_end {
            return Err(IonError::Truncated(self.position));
        }
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or(IonError::Truncated(self.position))?;
        self.position += 1;
        Ok(byte)
    }

    fn read_bytes(&mut self, length: usize, enclosing_end: usize) -> Result<&[u8], IonError> {
        let end = self.checked_end(length, enclosing_end)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(IonError::Truncated(self.position))?;
        self.position = end;
        Ok(bytes)
    }

    fn skip(&mut self, length: usize, enclosing_end: usize) -> Result<(), IonError> {
        self.position = self.checked_end(length, enclosing_end)?;
        Ok(())
    }

    fn checked_end(&self, length: usize, enclosing_end: usize) -> Result<usize, IonError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(IonError::Overflow(self.position))?;
        if end > enclosing_end || end > self.bytes.len() {
            return Err(IonError::Truncated(self.position));
        }
        Ok(end)
    }
}

fn read_unsigned_integer(bytes: &[u8], offset: usize) -> Result<u64, IonError> {
    if bytes.len() > 8 || bytes.first() == Some(&0) {
        return Err(IonError::IntegerOutOfRange(offset));
    }
    Ok(bytes
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)))
}

fn read_integer(bytes: &[u8], negative: bool, offset: usize) -> Result<i64, IonError> {
    let magnitude = read_unsigned_integer(bytes, offset)?;
    if negative {
        if magnitude == 1u64 << 63 {
            Ok(i64::MIN)
        } else {
            let magnitude =
                i64::try_from(magnitude).map_err(|_| IonError::IntegerOutOfRange(offset))?;
            magnitude
                .checked_neg()
                .ok_or(IonError::IntegerOutOfRange(offset))
        }
    } else {
        i64::try_from(magnitude).map_err(|_| IonError::IntegerOutOfRange(offset))
    }
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../../tests/unit/crates/folio-kfx/src/amazon/ion.rs"]
mod tests;
