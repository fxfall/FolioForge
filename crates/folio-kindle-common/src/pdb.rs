use thiserror::Error;

use crate::binary::{put_u16_be, put_u32_be, write_fixed_ascii};

#[derive(Clone, Debug)]
pub struct PdbRecord {
    pub attributes: u8,
    pub unique_id: u32,
    pub data: Vec<u8>,
}

impl PdbRecord {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            attributes: 0,
            unique_id: 0,
            data,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PdbDocument {
    pub name: String,
    pub type_code: [u8; 4],
    pub creator: [u8; 4],
    pub records: Vec<PdbRecord>,
    pub deterministic: bool,
}

#[derive(Debug, Error)]
pub enum PdbError {
    #[error("PDB contains too many records")]
    TooManyRecords,
    #[error("PDB record table or data exceeds the addressable range")]
    SizeOverflow,
    #[error("PDB binary construction failed: {0}")]
    Binary(#[from] crate::binary::BinaryError),
}

pub fn write_pdb(document: &PdbDocument) -> Result<Vec<u8>, PdbError> {
    if document.records.len() > usize::from(u16::MAX) {
        return Err(PdbError::TooManyRecords);
    }
    let record_table_size = document
        .records
        .len()
        .checked_mul(8)
        .ok_or(PdbError::SizeOverflow)?;
    let data_start = 78usize
        .checked_add(record_table_size)
        .ok_or(PdbError::SizeOverflow)?;
    let total_data = document
        .records
        .iter()
        .try_fold(0usize, |total, record| total.checked_add(record.data.len()))
        .ok_or(PdbError::SizeOverflow)?;
    let total_size = data_start
        .checked_add(total_data)
        .ok_or(PdbError::SizeOverflow)?;
    if total_size > u32::MAX as usize {
        return Err(PdbError::SizeOverflow);
    }

    let mut output = vec![0u8; total_size];
    write_fixed_ascii(&mut output[..32], &document.name);
    put_u16_be(&mut output, 32, 0)?; // attributes
    put_u16_be(&mut output, 34, 0)?; // version
                                     // Timestamps are zero in deterministic mode.  Non-deterministic timestamp
                                     // support is intentionally omitted until it is needed by a compatibility
                                     // corpus; zero is legal and makes semantic diffs stable.
    put_u32_be(&mut output, 36, 0)?;
    put_u32_be(&mut output, 40, 0)?;
    put_u32_be(&mut output, 44, 0)?;
    put_u32_be(&mut output, 48, 0)?;
    output[56..60].copy_from_slice(&document.type_code);
    output[60..64].copy_from_slice(&document.creator);
    put_u32_be(&mut output, 64, 0)?; // unique ID seed
    put_u32_be(&mut output, 68, 0)?; // next record list ID
    put_u16_be(&mut output, 76, document.records.len() as u16)?;

    let mut offset = data_start;
    for (index, record) in document.records.iter().enumerate() {
        let table_offset = 78usize
            .checked_add(index.checked_mul(8).ok_or(PdbError::SizeOverflow)?)
            .ok_or(PdbError::SizeOverflow)?;
        put_u32_be(&mut output, table_offset, offset as u32)?;
        output[table_offset + 4] = record.attributes;
        let uid = record.unique_id.max(index as u32) & 0x00ff_ffff;
        output[table_offset + 5] = ((uid >> 16) & 0xff) as u8;
        output[table_offset + 6] = ((uid >> 8) & 0xff) as u8;
        output[table_offset + 7] = (uid & 0xff) as u8;
        let end = offset
            .checked_add(record.data.len())
            .ok_or(PdbError::SizeOverflow)?;
        output[offset..end].copy_from_slice(&record.data);
        offset = end;
    }
    Ok(output)
}
