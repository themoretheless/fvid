//! Native VP9 diagnostic. Decode selected track to raw YUV, or `-` to validate without writing.
use fvid::{
    codec::{vp9, vp9_decoder::Decoder},
    container::webm::{Limits, WebmReader},
};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("expected WebM input")?;
    let output = args.next().ok_or("expected YUV output path or -")?;
    let max: usize = args
        .next()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(usize::MAX);
    let mut reader = WebmReader::open(BufReader::new(File::open(input)?), Limits::default())?;
    let track = reader
        .tracks
        .iter()
        .find(|t| t.codec == "V_VP9")
        .ok_or("missing VP9 track")?
        .number;
    let mut output: Option<BufWriter<Box<dyn Write>>> = if output == "-" {
        None
    } else if output == "/dev/stdout" {
        Some(BufWriter::new(Box::new(std::io::stdout())))
    } else {
        Some(BufWriter::new(Box::new(File::create(output)?)))
    };
    let mut decoder = Decoder::new(256 << 20);
    let mut shown = 0;
    let mut bytes = Vec::new();
    let start = std::time::Instant::now();
    for index in 0..reader.packets.len() {
        if reader.packets[index].track != track {
            continue;
        }
        let packet = reader.read_packet(index)?;
        for frame in vp9::frames(&packet)? {
            let decoded = decoder
                .decode(frame)
                .map_err(|e| format!("packet {index}: {e}"))?;
            if !decoded.header.show_frame {
                continue;
            }
            if let Some(output) = &mut output {
                bytes.clear();
                let picture = &decoded.picture;
                for (i, p) in picture.planes.iter().enumerate() {
                    let sub = if i == 0 { 1 } else { 2 };
                    let w = picture.size[0].div_ceil(sub) as usize;
                    let h = picture.size[1].div_ceil(sub) as usize;
                    for y in 0..h {
                        for x in 0..w {
                            let v = p.samples[y * p.width + x];
                            if picture.depth == 8 {
                                bytes.push(v as u8);
                            } else {
                                bytes.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                output.write_all(&bytes)?;
            }
            shown += 1;
            if shown % 100 == 0 {
                eprintln!(
                    "decoded {shown} frames in {:.2}s",
                    start.elapsed().as_secs_f64()
                );
            }
            if shown >= max {
                if let Some(output) = &mut output {
                    output.flush()?;
                }
                return Ok(());
            }
        }
    }
    if let Some(output) = &mut output {
        output.flush()?;
    }
    eprintln!(
        "decoded {shown} frames in {:.2}s",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
