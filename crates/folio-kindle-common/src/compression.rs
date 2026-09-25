use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compression {
    None,
    PalmDoc,
    HuffDic,
}

impl Compression {
    pub const fn code(self) -> u16 {
        match self {
            Self::None => 1,
            Self::PalmDoc => 2,
            Self::HuffDic => 17480,
        }
    }
}

#[derive(Debug, Error)]
pub enum CompressionError {
    #[error("HUFF/CDIC dictionary records are required for decompression")]
    HuffDicUnavailable,
    #[error("invalid HUFF/CDIC data: {0}")]
    HuffDicInvalid(String),
    #[error("HUFF/CDIC dictionary nesting exceeds the safety limit")]
    HuffDicDepthExceeded,
    #[error("HUFF/CDIC output exceeds the configured limit")]
    HuffDicOutputTooLarge,
    #[error("PalmDOC compressed stream is truncated")]
    Truncated,
    #[error("PalmDOC back-reference is invalid")]
    InvalidBackReference,
    #[error("PalmDOC decompressed stream exceeds the configured limit")]
    OutputTooLarge,
}

pub fn compress(input: &[u8], compression: Compression) -> Result<Vec<u8>, CompressionError> {
    match compression {
        Compression::None => Ok(input.to_vec()),
        Compression::PalmDoc => Ok(compress_palmdoc(input)),
        Compression::HuffDic => Err(CompressionError::HuffDicUnavailable),
    }
}

pub fn decompress(input: &[u8], compression: Compression) -> Result<Vec<u8>, CompressionError> {
    match compression {
        Compression::None => Ok(input.to_vec()),
        Compression::PalmDoc => decompress_palmdoc(input, usize::MAX),
        Compression::HuffDic => Err(CompressionError::HuffDicUnavailable),
    }
}

/// Whole-book HUFF/CDIC output allowance.  HUFF records use a shared
/// dictionary, so a per-record limit alone would allow a hostile file to
/// multiply the limit across many small records.
pub fn huffdic_text_budget(compressed_len: usize) -> usize {
    compressed_len
        .saturating_mul(64)
        .saturating_add(4 * 1024 * 1024)
}

const HUFF_MAX_DEPTH: usize = 32;
const HUFF_MAX_RECORD_OUTPUT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct HuffCode {
    length: u8,
    terminal: bool,
    max_code: u32,
}

#[derive(Clone, Debug)]
enum DictionaryEntry {
    Leaf(Vec<u8>),
    Node(Vec<u8>),
    Expanded(Vec<u8>),
}

/// A bounded decoder for the HUFF/CDIC representation used by many AZW3
/// files.  The decoder is deliberately constructed from the HUFF and CDIC
/// records rather than exposed through `decompress`, because the compressed
/// text record alone is not self-describing.
#[derive(Clone, Debug)]
pub struct HuffDicDecoder {
    first_table: Vec<HuffCode>,
    min_codes: Vec<u32>,
    max_codes: Vec<u32>,
    dictionary: Vec<DictionaryEntry>,
}

impl HuffDicDecoder {
    /// Parse one HUFF table and all CDIC dictionary records.
    pub fn new(huff: &[u8], cdics: &[&[u8]]) -> Result<Self, CompressionError> {
        let mut decoder = Self {
            first_table: Vec::with_capacity(256),
            min_codes: Vec::with_capacity(33),
            max_codes: Vec::with_capacity(33),
            dictionary: Vec::new(),
        };
        decoder.read_huff(huff)?;
        for cdic in cdics {
            decoder.read_cdic(cdic)?;
        }
        if decoder.dictionary.is_empty() {
            return Err(CompressionError::HuffDicInvalid(
                "no CDIC dictionary entries were found".to_owned(),
            ));
        }
        Ok(decoder)
    }

    /// Decode one compressed text record and charge emitted bytes against the
    /// caller's whole-book budget.
    pub fn decompress(
        &mut self,
        input: &[u8],
        shared_budget: &mut usize,
    ) -> Result<Vec<u8>, CompressionError> {
        let allowed = HUFF_MAX_RECORD_OUTPUT.min(*shared_budget);
        let mut remaining = allowed;
        let mut output = Vec::new();
        self.expand_bits(input, &mut output, 0, &mut remaining)?;
        *shared_budget = shared_budget.saturating_sub(allowed - remaining);
        Ok(output)
    }

    fn read_huff(&mut self, huff: &[u8]) -> Result<(), CompressionError> {
        if huff.len() < 24 || &huff[..8] != b"HUFF\0\0\0\x18" {
            return Err(CompressionError::HuffDicInvalid(
                "HUFF header is missing or truncated".to_owned(),
            ));
        }
        let first_offset = u32::from_be_bytes(huff[8..12].try_into().unwrap()) as usize;
        let second_offset = u32::from_be_bytes(huff[12..16].try_into().unwrap()) as usize;
        let first_end = first_offset
            .checked_add(256 * 4)
            .ok_or_else(|| invalid_huff("HUFF first table offset overflowed"))?;
        if first_end > huff.len() {
            return Err(invalid_huff("HUFF first table is truncated"));
        }

        for entry_index in 0..256 {
            let start = first_offset + entry_index * 4;
            let value = u32::from_be_bytes(huff[start..start + 4].try_into().unwrap());
            let length = (value & 0x1f) as u8;
            if length == 0 || length > 32 {
                return Err(invalid_huff("HUFF code length is outside 1..=32"));
            }
            let max_code_raw = value >> 8;
            let max_code = (max_code_raw.wrapping_add(1) << (32 - length)).wrapping_sub(1);
            self.first_table.push(HuffCode {
                length,
                terminal: value & 0x80 != 0,
                max_code,
            });
        }

        let second_end = second_offset
            .checked_add(32 * 8)
            .ok_or_else(|| invalid_huff("HUFF second table offset overflowed"))?;
        if second_end > huff.len() {
            return Err(invalid_huff("HUFF second table is truncated"));
        }
        self.min_codes.push(0);
        self.max_codes.push(0);
        for code_length in 1..=32 {
            let start = second_offset + (code_length - 1) * 8;
            let min_raw = u32::from_be_bytes(huff[start..start + 4].try_into().unwrap());
            let max_raw = u32::from_be_bytes(huff[start + 4..start + 8].try_into().unwrap());
            self.min_codes.push(min_raw << (32 - code_length));
            self.max_codes.push(
                max_raw
                    .wrapping_add(1)
                    .wrapping_shl((32 - code_length) as u32)
                    .wrapping_sub(1),
            );
        }
        Ok(())
    }

    fn read_cdic(&mut self, cdic: &[u8]) -> Result<(), CompressionError> {
        if cdic.len() < 16 || &cdic[..8] != b"CDIC\0\0\0\x10" {
            return Err(invalid_huff("CDIC header is missing or truncated"));
        }
        let phrase_count = u32::from_be_bytes(cdic[8..12].try_into().unwrap()) as usize;
        let bits = u32::from_be_bytes(cdic[12..16].try_into().unwrap());
        let table_count = 1usize
            .checked_shl(bits)
            .ok_or_else(|| invalid_huff("CDIC entry-width is too large"))?;
        let remaining = phrase_count.saturating_sub(self.dictionary.len());
        let count = remaining.min(table_count);
        let table_end = 16usize
            .checked_add(count.saturating_mul(2))
            .ok_or_else(|| invalid_huff("CDIC offset table overflowed"))?;
        if table_end > cdic.len() {
            return Err(invalid_huff("CDIC offset table is truncated"));
        }

        for entry_index in 0..count {
            let offset_start = 16 + entry_index * 2;
            let offset =
                u16::from_be_bytes(cdic[offset_start..offset_start + 2].try_into().unwrap())
                    as usize;
            let header_start = 16usize
                .checked_add(offset)
                .ok_or_else(|| invalid_huff("CDIC entry offset overflowed"))?;
            let header_end = header_start
                .checked_add(2)
                .ok_or_else(|| invalid_huff("CDIC entry header overflowed"))?;
            if header_end > cdic.len() {
                return Err(invalid_huff("CDIC entry header is truncated"));
            }
            let length_and_leaf =
                u16::from_be_bytes(cdic[header_start..header_end].try_into().unwrap());
            let length = usize::from(length_and_leaf & 0x7fff);
            let data_start = header_end;
            let data_end = data_start
                .checked_add(length)
                .ok_or_else(|| invalid_huff("CDIC entry length overflowed"))?;
            if data_end > cdic.len() {
                return Err(invalid_huff("CDIC entry data is truncated"));
            }
            let data = cdic[data_start..data_end].to_vec();
            if length_and_leaf & 0x8000 != 0 {
                self.dictionary.push(DictionaryEntry::Leaf(data));
            } else {
                self.dictionary.push(DictionaryEntry::Node(data));
            }
        }
        Ok(())
    }

    fn expand_bits(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        depth: usize,
        budget: &mut usize,
    ) -> Result<(), CompressionError> {
        if depth > HUFF_MAX_DEPTH {
            return Err(CompressionError::HuffDicDepthExceeded);
        }
        let mut padded = Vec::with_capacity(input.len().saturating_add(8));
        padded.extend_from_slice(input);
        padded.resize(input.len().saturating_add(8), 0);
        let mut position = 0usize;
        let mut bits_left = input.len().saturating_mul(8) as i64;
        let mut window = read_u64_be(&padded, 0);
        let mut window_bits = 32i32;

        while bits_left > 0 {
            if window_bits <= 0 {
                position = position.saturating_add(4);
                window = read_u64_be(&padded, position);
                window_bits += 32;
            }
            let code = ((window >> window_bits) & u32::MAX as u64) as u32;
            let first = self
                .first_table
                .get((code >> 24) as usize)
                .copied()
                .ok_or_else(|| invalid_huff("HUFF first table lookup failed"))?;
            let mut code_length = first.length;
            let mut max_code = first.max_code;
            if !first.terminal {
                while code_length < 32 && code < self.min_codes[usize::from(code_length)] {
                    code_length += 1;
                }
                max_code = self.max_codes[usize::from(code_length)];
            }
            window_bits -= i32::from(code_length);
            bits_left -= i64::from(code_length);
            if bits_left < 0 {
                break;
            }
            let dictionary_index = max_code
                .wrapping_sub(code)
                .wrapping_shr(u32::from(32 - code_length))
                as usize;
            let entry = self
                .dictionary
                .get(dictionary_index)
                .cloned()
                .ok_or_else(|| {
                    invalid_huff(format!(
                        "dictionary index {dictionary_index} is out of bounds"
                    ))
                })?;
            match entry {
                DictionaryEntry::Leaf(data) | DictionaryEntry::Expanded(data) => {
                    take_huff_budget(budget, data.len())?;
                    output.extend_from_slice(&data);
                }
                DictionaryEntry::Node(data) => {
                    let mut expanded = Vec::new();
                    self.expand_bits(&data, &mut expanded, depth + 1, budget)?;
                    output.extend_from_slice(&expanded);
                    if let Some(slot) = self.dictionary.get_mut(dictionary_index) {
                        *slot = DictionaryEntry::Expanded(expanded);
                    }
                }
            }
        }
        Ok(())
    }
}

fn invalid_huff(message: impl Into<String>) -> CompressionError {
    CompressionError::HuffDicInvalid(message.into())
}

fn take_huff_budget(budget: &mut usize, size: usize) -> Result<(), CompressionError> {
    *budget = budget
        .checked_sub(size)
        .ok_or(CompressionError::HuffDicOutputTooLarge)?;
    Ok(())
}

fn read_u64_be(data: &[u8], start: usize) -> u64 {
    let Some(bytes) = data.get(start..start.saturating_add(8)) else {
        return 0;
    };
    u64::from_be_bytes(bytes.try_into().unwrap())
}

/// Decode PalmDOC's compact stream with an explicit output bound.  The bound
/// is important because this function is also used on untrusted input files.
pub fn decompress_palmdoc(input: &[u8], max_output: usize) -> Result<Vec<u8>, CompressionError> {
    let mut output = Vec::with_capacity(input.len());
    let mut cursor = 0usize;
    while cursor < input.len() {
        let byte = input[cursor];
        cursor += 1;
        match byte {
            0x00..=0x08 => {
                let length = usize::from(byte);
                let end = cursor
                    .checked_add(length)
                    .ok_or(CompressionError::OutputTooLarge)?;
                if end > input.len() {
                    return Err(CompressionError::Truncated);
                }
                if output
                    .len()
                    .checked_add(length)
                    .is_none_or(|size| size > max_output)
                {
                    return Err(CompressionError::OutputTooLarge);
                }
                output.extend_from_slice(&input[cursor..end]);
                cursor = end;
            }
            0x09..=0x7f => {
                if output.len() == max_output {
                    return Err(CompressionError::OutputTooLarge);
                }
                output.push(byte);
            }
            0x80..=0xbf => {
                let second = *input.get(cursor).ok_or(CompressionError::Truncated)?;
                cursor += 1;
                let distance = ((usize::from(byte & 0x3f)) << 5) | usize::from(second >> 3);
                let length = usize::from(second & 0x07) + 3;
                if distance == 0 || distance > output.len() {
                    return Err(CompressionError::InvalidBackReference);
                }
                if output
                    .len()
                    .checked_add(length)
                    .is_none_or(|size| size > max_output)
                {
                    return Err(CompressionError::OutputTooLarge);
                }
                for _ in 0..length {
                    let index = output.len() - distance;
                    let value = output[index];
                    output.push(value);
                }
            }
            0xc0..=0xff => {
                if output
                    .len()
                    .checked_add(2)
                    .is_none_or(|size| size > max_output)
                {
                    return Err(CompressionError::OutputTooLarge);
                }
                output.push(b' ');
                output.push(byte ^ 0x80);
            }
        }
    }
    Ok(output)
}

/// PalmDOC's compact back-reference form.  Literal bytes are retained when a
/// sequence cannot be represented; this is intentionally small and safe, and
/// HUFF/CDIC remains an explicit future capability rather than a fake option.
pub fn compress_palmdoc(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        if input[index] == b' '
            && index + 1 < input.len()
            && (0x40..=0x7f).contains(&input[index + 1])
        {
            output.push(0xc0 | input[index + 1]);
            index += 2;
            continue;
        }

        let mut best_distance = 0usize;
        let mut best_length = 0usize;
        let window_start = index.saturating_sub(2047);
        for candidate in window_start..index {
            let mut length = 0usize;
            while length < 10
                && index + length < input.len()
                && input[candidate + length] == input[index + length]
            {
                length += 1;
                if candidate + length >= index {
                    break;
                }
            }
            if length >= 3 && length > best_length {
                best_distance = index - candidate;
                best_length = length;
            }
        }
        if best_length >= 3 && best_distance <= 2047 {
            output.push(0x80 | ((best_distance >> 5) as u8 & 0x3f));
            output.push(((best_distance as u8 & 0x1f) << 3) | ((best_length - 3) as u8 & 0x07));
            index += best_length;
        } else {
            let byte = input[index];
            if !(0x09..=0x7f).contains(&byte) {
                // PalmDOC reserves 0x00..0x08 as an escape for a short run of
                // raw bytes.  Use it for UTF-8 continuation bytes and other
                // values that would otherwise be parsed as back-references.
                let start = index;
                let mut length = 0usize;
                while index < input.len() && length < 8 && !(0x09..=0x7f).contains(&input[index]) {
                    index += 1;
                    length += 1;
                }
                output.push(length as u8);
                output.extend_from_slice(&input[start..index]);
            } else {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-kindle-common/src/compression.rs"]
mod tests;
