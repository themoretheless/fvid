//! A PNG writer small enough to keep next to the player.
//!
//! The snapshot key hands a frame to this module, and a snapshot the user has
//! asked for must not depend on an image library the player does not carry. The
//! file is therefore written with stored DEFLATE blocks: no compression, only
//! the framing a conforming decoder reads.

use crate::{Result, invalid};

/// The eight bytes every PNG file starts with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Largest run of bytes a stored DEFLATE block can carry.
const BLOCK: usize = 65_535;

/// The CRC-32 polynomial of the standard table, applied byte at a time.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

/// Write an 8-bit truecolour image as a PNG file. `rgb` holds three bytes per
/// pixel, row by row, and `width * height * 3` has to be its length.
pub fn png(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return Err(invalid("snapshot pixels do not match their size"));
    }
    let row = width
        .checked_mul(3)
        .and_then(|row| row.checked_add(1))
        .ok_or_else(|| invalid("snapshot row overflows"))?;
    let raw_len = row
        .checked_mul(height)
        .ok_or_else(|| invalid("snapshot is too large"))?;
    // The zlib stream wraps the filtered rows, so account for its two header
    // bytes, the block headers and the trailing checksum.
    let blocks = raw_len.div_ceil(BLOCK).max(1);
    let idat_len = raw_len + 6 + blocks * 5;
    if idat_len > u32::MAX as usize {
        return Err(invalid("snapshot is too large"));
    }
    let mut filtered = Vec::with_capacity(raw_len);
    for line in rgb.chunks_exact(row - 1) {
        filtered.push(0); // No filter: the rows are stored as they came.
        filtered.extend_from_slice(line);
    }
    let mut out = Vec::with_capacity(idat_len + 64);
    out.extend_from_slice(&SIGNATURE);
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    // Bit depth 8, colour type 2 (truecolour), no compression, filter or interlace.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    let mut stream = Vec::with_capacity(idat_len);
    stream.extend_from_slice(&[0x78, 0x01]); // Deflate, smallest window, no dictionary.
    for (index, part) in filtered.chunks(BLOCK).enumerate() {
        let last = index * BLOCK + part.len() == filtered.len();
        stream.push(u8::from(last));
        stream.extend_from_slice(&(part.len() as u16).to_le_bytes());
        stream.extend_from_slice(&(!(part.len() as u16)).to_le_bytes());
        stream.extend_from_slice(part);
    }
    stream.extend_from_slice(&adler32(&filtered).to_be_bytes());
    chunk(&mut out, b"IDAT", &stream);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// One PNG chunk: length, type, payload, and the CRC over type and payload.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let mut crc = !0u32;
    crc = update_crc(crc, kind);
    crc = update_crc(crc, payload);
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn update_crc(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc = CRC_TABLE[usize::from((crc as u8) ^ byte)] ^ (crc >> 8);
    }
    crc
}

/// Checksum of the payload the zlib stream wraps.
fn adler32(bytes: &[u8]) -> u32 {
    const MODULO: u32 = 65_521;
    // Largest run that cannot overflow the 32-bit sums between reductions.
    const GROUP: usize = 5_552;
    let (mut a, mut b) = (1u32, 0u32);
    for part in bytes.chunks(GROUP) {
        for byte in part {
            a = a.wrapping_add(u32::from(*byte));
            b = b.wrapping_add(a);
        }
        a %= MODULO;
        b %= MODULO;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::{SIGNATURE, adler32, png};

    /// Walk the chunk stream the writer produced, checking every CRC, and
    /// return the payload of each chunk by type.
    fn chunks(file: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        assert_eq!(&file[..8], &SIGNATURE);
        let mut at = 8;
        let mut found = Vec::new();
        while at < file.len() {
            let len = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = file[at + 4..at + 8].try_into().unwrap();
            let payload = file[at + 8..at + 8 + len].to_vec();
            let crc = u32::from_be_bytes(file[at + 8 + len..at + 12 + len].try_into().unwrap());
            let mut sum = !0u32;
            for byte in kind.iter().chain(&payload) {
                sum = super::CRC_TABLE[usize::from((sum as u8) ^ byte)] ^ (sum >> 8);
            }
            assert_eq!(crc, !sum, "CRC of chunk {kind:?}");
            found.push((kind, payload));
            at += 12 + len;
        }
        found
    }

    /// Undo the stored blocks and the row filters, giving the pixels back.
    fn pixels(payload: &[u8], width: usize, height: usize) -> Vec<u8> {
        assert_eq!(&payload[..2], &[0x78, 0x01]);
        let mut raw = Vec::new();
        let mut at = 2;
        loop {
            let header = payload[at];
            let last = header & 1 != 0;
            assert_eq!(header & 0x06, 0, "only stored blocks are written");
            at += 1;
            let len = u16::from_le_bytes(payload[at..at + 2].try_into().unwrap()) as usize;
            let inverted = u16::from_le_bytes(payload[at + 2..at + 4].try_into().unwrap());
            assert_eq!(inverted, !(len as u16));
            at += 4;
            raw.extend_from_slice(&payload[at..at + len]);
            at += len;
            if last {
                break;
            }
        }
        assert_eq!(&payload[at..at + 4], adler32(&raw).to_be_bytes());
        let row = width * 3 + 1;
        let mut rgb = Vec::new();
        for line in raw.chunks_exact(row) {
            assert_eq!(line[0], 0, "rows are stored unfiltered");
            rgb.extend_from_slice(&line[1..]);
        }
        assert_eq!(rgb.len(), width * height * 3);
        rgb
    }

    #[test]
    fn a_small_picture_survives_the_round_trip() {
        // Four pixels, one per corner colour, in the order the player holds them.
        let rgb = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let file = png(2, 2, &rgb).expect("encoded");
        let found = chunks(&file);
        let kinds: Vec<[u8; 4]> = found.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, [*b"IHDR", *b"IDAT", *b"IEND"]);
        let header = &found[0].1;
        assert_eq!(&header[..8], &[0, 0, 0, 2, 0, 0, 0, 2]);
        assert_eq!(&header[8..], &[8, 2, 0, 0, 0]);
        assert_eq!(pixels(&found[1].1, 2, 2), rgb);
    }

    #[test]
    fn a_picture_over_one_block_writes_each_of_them() {
        // 30 000 pixels of one colour: 90 000 byte rows spread over two blocks.
        let width = 300;
        let height = 100;
        let rgb = vec![7u8; width * height * 3];
        let file = png(width, height, &rgb).expect("encoded");
        let found = chunks(&file);
        let idat = &found.iter().find(|(kind, _)| kind == b"IDAT").unwrap().1;
        // The first block carries the maximum run, so its length field reads 65 535.
        assert_eq!(&idat[3..5], &[0xff, 0xff]);
        assert_eq!(pixels(idat, width, height), rgb);
    }

    #[test]
    fn pixels_that_do_not_match_their_size_are_refused() {
        assert!(png(0, 1, &[]).is_err());
        assert!(png(1, 1, &[1, 2]).is_err());
        assert!(png(2, 1, &[1, 2, 3]).is_err());
        assert!(png(1, 1, &[1, 2, 3]).is_ok());
    }

    #[test]
    fn the_checksums_match_their_reference_vectors() {
        // The published CRC-32 and Adler-32 values for these inputs.
        let mut crc = !0u32;
        crc = super::update_crc(crc, b"123456789");
        assert_eq!(!crc, 0xCBF4_3926);
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"abc"), 0x024D_0127);
        assert_eq!(adler32(&[0xff; 6_000]), 0xA497_59EA);
    }
}
