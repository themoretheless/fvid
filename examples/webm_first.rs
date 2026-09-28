//! Time the player's own open path on a slow source, head to picture.
use fvid::playback_native::NativeReader;
use std::{fs::File, io::BufReader, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("expected media path")?;
    let start = Instant::now();
    let mut reader = NativeReader::without_memory_limit(BufReader::with_capacity(
        1 << 20,
        File::open(&path)?,
    ))?;
    println!("reader built in {:?}", start.elapsed());
    let start = Instant::now();
    reader.read_frame()?;
    println!(
        "first picture in {:?} dims={:?} duration={:?}",
        start.elapsed(),
        reader.dimensions(),
        reader.duration()
    );
    Ok(())
}
