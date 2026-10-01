//! Owned WAVE extensible float-PCM headers, independent of libav.
fn error(message:&str)->String {message.into()}
pub fn default_pcm_mask(channels:u16) -> Result<u32,String> {
    Ok(match channels {
        1 => 0x4, 2 => 0x3, 3 => 0x7, 4 => 0x107, 5 => 0x37, 6 => 0x3f,
        7..=64 => 0,
        _ => return Err(error("unsupported WAV channel count")),
    })
}
pub fn float_wav_header_with_mask(sample_rate:u32,channels:u16,sample_frames:u64,mask:u32) -> Result<Vec<u8>,String> {
    if !(1..=64).contains(&channels) || (mask != 0 && mask.count_ones()!=u32::from(channels)) {return Err(error("invalid WAV channel mask"));}
    let align = channels * 4;
    let bytes = sample_frames.checked_mul(u64::from(align))
        .and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= u32::MAX - 72)
        .ok_or_else(|| error("WAV exceeds RIFF size limit"))?;
    let rate = sample_rate.checked_mul(u32::from(align))
        .ok_or_else(|| error("WAV byte rate overflow"))?;
    let mut header = Vec::with_capacity(80);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(bytes + 72).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&40u32.to_le_bytes());
    header.extend_from_slice(&0xfffeu16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&align.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&22u16.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&mask.to_le_bytes());
    header.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
    header.extend_from_slice(b"fact");
    header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&(sample_frames as u32).to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&bytes.to_le_bytes());
    Ok(header)
}

