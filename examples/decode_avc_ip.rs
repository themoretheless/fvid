//! Decode supported AVC I/P/B pictures in presentation order using only FVid.
use fvid::{container::mp4::Limits, playback_mp4::Mp4AvcReader};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: decode_avc_ip INPUT.mp4 OUTPUT.yuv".into());
    }
    let mut source = Mp4AvcReader::open(
        BufReader::new(File::open(&args[0])?),
        Limits::default(),
        256 << 20,
    )?;
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    let mut frames = 0;
    let mut first = None;
    while let Some(frame) = source.read_frame()? {
        frame.picture.write_planar(&mut output)?;
        if first.is_none() {
            first = Some(frame);
        }
        frames += 1;
    }
    source.rewind();
    if let Some(first) = first {
        let replay = source.read_frame()?.ok_or("missing frame after rewind")?;
        if first.presentation_time != replay.presentation_time
            || first.duration != replay.duration
            || first.picture.y != replay.picture.y
            || first.picture.cb != replay.picture.cb
            || first.picture.cr != replay.picture.cr
        {
            return Err("rewind changed the first decoded frame".into());
        }
    }
    output.flush()?;
    println!("frames={frames}");
    Ok(())
}
