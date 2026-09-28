//! Decode low-overhead AV1 OBUs or AV1 WebM into planar YUV using FVid only.
use fvid::{
    codec::av1_decoder::Decoder,
    container::webm::{Limits, WebmReader},
};
use std::{
    error::Error,
    fs::File,
    io::{Read, Write},
};
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("input path required")?;
    let output = args.next().ok_or("output path or - required")?;
    let limit = args
        .next()
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    let mut writer: Box<dyn Write> = if output == "-" {
        Box::new(std::io::sink())
    } else if output == "/dev/stdout" {
        Box::new(std::io::stdout())
    } else {
        Box::new(File::create(output)?)
    };
    let mut decoder = Decoder::new(256 << 20);
    let mut frames = 0usize;
    let mut emit = |data: &[u8]| -> Result<bool, Box<dyn Error>> {
        if frames >= limit {
            return Ok(true);
        }
        for decoded in decoder.decode_packet(data)? {
            if decoded.show && frames < limit {
                for (p, plane) in decoded
                    .picture
                    .planes
                    .iter()
                    .enumerate()
                    .take(if decoded.color.monochrome { 1 } else { 3 })
                {
                    let sub = usize::from(p > 0);
                    let w = (decoded.picture.size[0] as usize).div_ceil(1 << sub);
                    let h = (decoded.picture.size[1] as usize).div_ceil(1 << sub);
                    for y in 0..h {
                        for &v in &plane.samples[y * plane.width..y * plane.width + w] {
                            if decoded.picture.depth == 8 {
                                writer.write_all(&[v as u8])?;
                            } else {
                                writer.write_all(&v.to_le_bytes())?;
                            }
                        }
                    }
                }
                frames += 1;
            }
        }
        Ok(frames >= limit)
    };
    let mut file = File::open(&input)?;
    let mut prefix = [0; 4];
    file.read_exact(&mut prefix)?;
    if prefix == [0x1a, 0x45, 0xdf, 0xa3] {
        let mut reader = WebmReader::open(File::open(input)?, Limits::default())?;
        reader.scan_all()?;
        let track = reader
            .tracks
            .iter()
            .find(|t| t.kind == 1 && t.codec == "V_AV1")
            .ok_or("no AV1 video track")?
            .number;
        for i in 0..reader.packets.len() {
            if reader.packets[i].track == track {
                if emit(&reader.read_packet(i)?)? {
                    break;
                }
            }
        }
    } else {
        let bytes = std::fs::read(input)?;
        // Feed complete sequence/temporal/frame OBUs separately, preserving state.
        let mut offset = 0;
        for obu in fvid::codec::av1::Obus::new(&bytes) {
            let obu = obu?;
            let end = obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
            if emit(&bytes[offset..end])? {
                break;
            }
            offset = end;
        }
    }
    eprintln!("Decoded {frames} AV1 frames");
    Ok(())
}
