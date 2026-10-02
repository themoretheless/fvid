fn size(value: u64) -> Result<Vec<u8>> {
    for width in 1..=8 {
        if value < (1u64 << (7 * width)) - 1 {
            let encoded = (value | (1u64 << (7 * width))).to_be_bytes();
            return Ok(encoded[8 - width..].to_vec());
        }
    }
    Err(invalid("Matroska element exceeds size range"))
}
fn head(output: &mut impl Write, id: u32, length: u64) -> Result<()> {
    let bytes = id.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .ok_or_else(|| invalid("zero EBML ID"))?;
    output.write_all(&bytes[start..])?;
    output.write_all(&size(length)?)?;
    Ok(())
}
fn element(id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    head(&mut out, id, data.len() as u64)?;
    out.extend_from_slice(data);
    Ok(out)
}
fn uint(id: u32, value: u64) -> Result<Vec<u8>> {
    let b = value.to_be_bytes();
    element(id, &b[b.iter().position(|&v| v != 0).unwrap_or(7)..])
}
