//! Decode MP4/HEVC with the native FVid library; optionally dump cropped YUV planes.
use fvid::{
    codec::hevc_decoder::HevcDecoder,
    container::mp4::{Limits, Mp4Reader},
};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .ok_or("usage: hevc_decode INPUT.mp4 [COUNT] [OUTPUT_DIR]")?;
    let limit = args
        .next()
        .map(|v| v.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    let output = args.next().map(PathBuf::from);
    let mut reader = Mp4Reader::open(BufReader::new(File::open(input)?), Limits::default())?;
    let track = reader
        .tracks()
        .iter()
        .position(|t| matches!(&t.codec, b"hvc1" | b"hev1"))
        .ok_or("no HEVC track")?;
    let mut decoder =
        HevcDecoder::from_configuration(&reader.tracks()[track].configuration, usize::MAX)?;
    let mut packet = Vec::new();
    let mut count = 0;
    let start = std::time::Instant::now();
    for sample in 0..reader.tracks()[track].samples.len().min(limit) {
        reader.read_packet(track, sample, &mut packet)?;
        let decoded = decoder
            .decode_packet(&packet)
            .map_err(|e| format!("sample {sample}: {e}"))?;
        if let Some(decoded) = decoded {
            let picture = decoded.picture;
            let mut hash = 0xcbf29ce484222325u64;
            let mut out = output
                .as_ref()
                .map(|dir| {
                    File::create(dir.join(format!("{sample:06}-poc-{}.yuv", decoded.poc)))
                        .map(BufWriter::new)
                })
                .transpose()?;
            for (c, plane) in picture.planes.iter().enumerate() {
                let shift = usize::from(c != 0);
                let stride = picture.dimensions[0] as usize >> shift;
                let [left, right, top, bottom] = picture.crop.map(|v| v as usize >> shift);
                let height = picture.dimensions[1] as usize >> shift;
                for y in top..height - bottom {
                    for &value in &plane.samples()[y * stride + left..(y + 1) * stride - right] {
                        let bytes = value.to_le_bytes();
                        let bytes = if picture.depth[shift] == 8 {
                            &bytes[..1]
                        } else {
                            &bytes[..]
                        };
                        for &v in bytes {
                            hash = (hash ^ u64::from(v)).wrapping_mul(0x100000001b3);
                        }
                        if let Some(out) = &mut out {
                            out.write_all(bytes)?;
                        }
                    }
                }
            }
            if let Some(out) = &mut out {
                out.flush()?;
            }
            count += 1;
            println!("sample={sample} poc={} hash={hash:016x}", decoded.poc);
        }
    }
    eprintln!("decoded {count} frames in {:?}", start.elapsed());
    Ok(())
}
