//! Coded frame counts without PCM allocation; decoder validation remains authoritative.
pub fn frames(
    codec: &[u8; 4],
    configuration: &[u8],
    rate: u32,
    channels: u16,
    bits: u16,
    bytes: usize,
    payload: Option<&[u8]>,
) -> Result<u64, String> {
    let ch = usize::from(channels);
    if ch == 0 {
        return Err("zero MP4 coded channels".into());
    }
    let blocks = |block: usize, n: usize| -> Result<u64, String> {
        if block == 0 || bytes == 0 || !bytes.is_multiple_of(block) {
            return Err("incomplete MP4 coded audio block".into());
        }
        (bytes / block)
            .checked_mul(n)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| "MP4 coded frame count overflow".into())
    };
    match codec {
        b"mp4a" => {
            let asc = crate::owned_codec_config::aac_specific_config(configuration)
                .map_err(|e| e.to_string())?;
            let parsed = crate::owned_aac::config::AudioSpecificConfig::parse(asc)
                .map_err(|e| e.to_string())?;
            let n = u64::from(parsed.core.frame_samples) * u64::from(rate);
            let scale = u64::from(parsed.core.sample_rate);
            if scale == 0 || n % scale != 0 {
                return Err("invalid MP4 AAC coded clock".into());
            }
            Ok(n / scale)
        }
        b"alac" => crate::owned_alac::AlacDecoder::packet_samples(
            configuration,
            rate,
            channels,
            payload.ok_or("ALAC coded count requires its packet header")?,
        )
        .map(|n| n as u64)
        .map_err(|e| e.to_string()),
        b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" => blocks(
            ch.checked_mul(usize::from(bits).div_ceil(8))
                .ok_or("MP4 PCM stride overflow")?,
            1,
        ),
        b"MAC3" => blocks(ch * 2, 6),
        b"MAC6" => blocks(ch, 6),
        b"ima4" => blocks(ch * 34, 64),
        b"ms\0\x11" | b"ms\0\x02" => {
            let block = configuration
                .get(12..14)
                .ok_or("incomplete ADPCM block configuration")?;
            let block = usize::from(u16::from_le_bytes(block.try_into().unwrap()));
            let header = ch * if codec == b"ms\0\x11" { 4 } else { 7 };
            let n = block
                .checked_sub(header)
                .and_then(|v| v.checked_mul(2))
                .map(|n| n / ch + if codec == b"ms\0\x11" { 1 } else { 2 })
                .ok_or("invalid ADPCM coded geometry")?;
            blocks(block, n)
        }
        _ => Err("unsupported MP4 coded sample clock".into()),
    }
}
