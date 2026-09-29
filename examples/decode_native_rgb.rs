//! Exercise the same native source used by the GUI without a window.
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !matches!(args.len(), 2 | 3) {
        return Err("usage: decode_native_rgb INPUT OUTPUT.rgb [TIMESTAMPS.json]".into());
    }
    let mut reader = fvid::playback_native::NativeReader::software(
        BufReader::new(File::open(&args[0])?),
        256 << 20,
    )?;
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    let mut first = None;
    let mut frames = 0;
    let mut timestamps = Vec::new();
    while reader.read_frame()? {
        if first.is_none() {
            first = Some((
                reader.rgb().to_vec(),
                reader.dimensions(),
                reader.frame_period(),
            ));
        }
        output.write_all(reader.rgb())?;
        if args.len() == 3 {
            let (start, _, scale) = reader
                .frame_interval()
                .ok_or("frame has no presentation interval")?;
            let ns = start
                .checked_mul(1_000_000_000)
                .ok_or("timestamp overflow")?
                .div_ceil(u128::from(scale));
            timestamps.push(u64::try_from(ns)?);
        }
        frames += 1;
    }
    reader.rewind()?;
    if let Some((rgb, dimensions, period)) = first {
        if !reader.read_frame()?
            || reader.rgb() != rgb
            || reader.dimensions() != dimensions
            || reader.frame_period() != period
        {
            return Err("native RGB rewind mismatch".into());
        }
    }
    output.flush()?;
    if args.len() == 3 {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?;
        serde_json::to_writer(file, &timestamps)?;
    }
    println!("frames={frames}");
    Ok(())
}
