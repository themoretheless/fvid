//! B-picture oracle harness. Buffers a bounded short clip and sorts by MP4 PTS.
use fvid::{
    codec::avc_decoder::AvcDecoder,
    container::mp4::{Limits, Mp4Reader},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: decode_avc_reordered INPUT.mp4 OUTPUT.yuv".into());
    }
    let mut demux = Mp4Reader::open(BufReader::new(File::open(&args[0])?), Limits::default())?;
    let track = demux
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide" && t.codec == *b"avc1")
        .ok_or("no AVC track")?;
    let mut decoder = AvcDecoder::new(&demux.tracks()[track].configuration, 256 << 20)?;
    let mut packet = Vec::new();
    let mut frames = Vec::new();
    let mut bytes = 0usize;
    let total = demux.tracks()[track].samples.len();
    for index in 0..total {
        let pts = demux.tracks()[track]
            .samples
            .get(index)
            .ok_or("sample index out of range")?
            .pts;
        demux.read_packet(track, index, &mut packet)?;
        if let Some(picture) = decoder.decode_order(&packet)? {
            bytes = bytes
                .checked_add((picture.y.len() + picture.cb.len() + picture.cr.len()) * 2)
                .ok_or("output size overflow")?;
            if bytes > 256 << 20 {
                return Err("oracle clip exceeds 256 MiB output budget".into());
            }
            frames.push((pts, picture));
        }
    }
    frames.sort_by_key(|(pts, _)| *pts);
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    for (_, picture) in &frames {
        picture.write_planar(&mut output)?;
    }
    output.flush()?;
    println!("frames={}", frames.len());
    Ok(())
}
