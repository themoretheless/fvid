//! Exercise the same native source used by the GUI without a window.
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: decode_native_rgb INPUT OUTPUT.rgb".into());
    }
    let mut reader =
        fvid::playback_native::NativeReader::new(BufReader::new(File::open(&args[0])?), 256 << 20)?;
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    let mut first = None;
    let mut frames = 0;
    while reader.read_frame()? {
        if first.is_none() {
            first = Some((
                reader.rgb().to_vec(),
                reader.dimensions(),
                reader.frame_period(),
            ));
        }
        output.write_all(reader.rgb())?;
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
    println!("frames={frames}");
    Ok(())
}
