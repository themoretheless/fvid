//! Bounded, checksum-verified RFC 1950 frame decompression.
use miniz_oxide::inflate::{TINFLStatus, core::{decompress, inflate_flags, DecompressorOxide}};

/// Restore one complete zlib stream with a logical output limit. Input bytes
/// are separately admitted by the container. A one-byte output sentinel lets
/// an exactly full frame still finish reading its checksum.
pub fn decode(input: &[u8], maximum: usize) -> Result<Vec<u8>, &'static str> {
    let limit = maximum.saturating_add(1);
    let mut state = DecompressorOxide::default();
    let mut output = Vec::new();
    let (mut consumed, mut written) = (0, 0);
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    loop {
        if output.len() == written {
            let next = output.len().saturating_mul(2).max(64).min(limit);
            if next <= output.len() { return Err("WebM packet exceeds budget"); }
            output.try_reserve_exact(next - output.len())
                .map_err(|_| "Matroska zlib buffer allocation failed")?;
            output.resize(next, 0);
        }
        let (status, read, made) = decompress(&mut state, &input[consumed..], &mut output, written, flags);
        consumed += read;
        written += made;
        if written > maximum { return Err("WebM packet exceeds budget"); }
        match status {
            TINFLStatus::Done => {
                if consumed != input.len() { return Err("trailing data after Matroska zlib stream"); }
                output.truncate(written);
                return Ok(output);
            }
            TINFLStatus::HasMoreOutput => {
                // The next iteration grows the admitted output buffer.
                if made == 0 && output.len() != written {
                    return Err("invalid Matroska zlib stream");
                }
            }
            _ => return Err("invalid or truncated Matroska zlib stream"),
        }
    }
}

/// Additional fixed inflater state beside the encoded and decoded buffers.
pub fn state_bytes() -> usize { std::mem::size_of::<DecompressorOxide>() }
