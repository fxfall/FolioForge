use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinaryError {
    #[error("unexpected end of input while reading {what}")]
    UnexpectedEof { what: &'static str },
    #[error("invalid {what}: {value}")]
    Invalid { what: &'static str, value: String },
    #[error("integer overflow while calculating {what}")]
    Overflow { what: &'static str },
}

pub fn put_u16_be(output: &mut [u8], offset: usize, value: u16) -> Result<(), BinaryError> {
    let end = offset
        .checked_add(2)
        .ok_or(BinaryError::Overflow { what: "u16 offset" })?;
    let target = output
        .get_mut(offset..end)
        .ok_or(BinaryError::UnexpectedEof { what: "u16" })?;
    target.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

pub fn put_u32_be(output: &mut [u8], offset: usize, value: u32) -> Result<(), BinaryError> {
    let end = offset
        .checked_add(4)
        .ok_or(BinaryError::Overflow { what: "u32 offset" })?;
    let target = output
        .get_mut(offset..end)
        .ok_or(BinaryError::UnexpectedEof { what: "u32" })?;
    target.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

pub fn read_u16_be(input: &[u8], offset: usize) -> Result<u16, BinaryError> {
    let end = offset
        .checked_add(2)
        .ok_or(BinaryError::Overflow { what: "u16 offset" })?;
    let bytes = input
        .get(offset..end)
        .ok_or(BinaryError::UnexpectedEof { what: "u16" })?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub fn read_u32_be(input: &[u8], offset: usize) -> Result<u32, BinaryError> {
    let end = offset
        .checked_add(4)
        .ok_or(BinaryError::Overflow { what: "u32 offset" })?;
    let bytes = input
        .get(offset..end)
        .ok_or(BinaryError::UnexpectedEof { what: "u32" })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn write_fixed_ascii(output: &mut [u8], value: &str) {
    output.fill(0);
    let bytes = value.as_bytes();
    let length = bytes.len().min(output.len());
    output[..length].copy_from_slice(&bytes[..length]);
}
