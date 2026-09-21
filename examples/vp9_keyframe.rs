//! Native intra-frame diagnostic; writes planar little-endian YUV to a chosen path.
use fvid::{
    codec::{
        vp9::{HeaderState, frames},
        vp9_picture::decode_intra,
        vp9_probs::{CompressedHeader, Probabilities},
    },
    container::webm::{Limits, WebmReader},
};
use std::{
    fs::File,
    io::{BufReader, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("expected input WebM")?;
    let output = args.next().ok_or("expected output YUV")?;
    let mut reader = WebmReader::open(BufReader::new(File::open(input)?), Limits::default())?;
    let track = reader
        .tracks
        .iter()
        .find(|t| t.codec == "V_VP9")
        .ok_or("missing VP9 track")?
        .number;
    let index = reader
        .packets
        .iter()
        .position(|p| p.track == track && p.keyframe)
        .ok_or("missing keyframe")?;
    let packet = reader.read_packet(index)?;
    let chunks = frames(&packet)?;
    let frame = chunks[0];
    let header = HeaderState::default().parse(frame)?;
    let compressed = CompressedHeader::parse(frame, &header, &Probabilities::default())?;
    let picture = decode_intra(frame, &header, &compressed, 256 << 20)?;
    let mut out = File::create(output)?;
    for (i, p) in picture.planes.iter().enumerate() {
        let sub = if i == 0 { 1 } else { 2 };
        let w = picture.size[0].div_ceil(sub) as usize;
        let h = picture.size[1].div_ceil(sub) as usize;
        for y in 0..h {
            for x in 0..w {
                let v = p.samples[y * p.width + x];
                if picture.depth == 8 {
                    out.write_all(&[v as u8])?;
                } else {
                    out.write_all(&v.to_le_bytes())?;
                }
            }
        }
    }
    println!(
        "decoded native VP9 keyframe {}x{} {}-bit",
        picture.size[0], picture.size[1], picture.depth
    );
    Ok(())
}
