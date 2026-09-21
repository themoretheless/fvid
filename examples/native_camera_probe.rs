//! Probe file-to-camera BGRA output without installing an OS camera extension.
use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraTick, LatestFrame, NativeCameraSource},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: native_camera_probe INPUT OUTPUT.bgra".into());
    }
    let mut reader = NativeReader::new(BufReader::new(File::open(&args[0])?), 256 << 20)?;
    if !reader.read_frame()? {
        return Err("empty camera input".into());
    }
    let [w, h] = reader.dimensions();
    reader.rewind()?;
    let mut source = NativeCameraSource::new(reader);
    let output = LatestFrame::new(w, h, 64 << 20)?;
    let mut pixels = vec![0; w * h * 4];
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    for (sequence, media_time_ns) in [0, 40_000_000, 400_000_000, 0].into_iter().enumerate() {
        let tick = CameraTick {
            sequence: sequence as u64,
            host_time_ns: sequence as u64 + 1,
            media_time_ns,
        };
        if !source.publish(tick, &output)? {
            return Err("empty camera output".into());
        }
        if output.copy_latest(None, &mut pixels)? != Some(tick) {
            return Err("camera timestamp mismatch".into());
        }
        writer.write_all(&pixels)?;
    }
    writer.flush()?;
    Ok(())
}
