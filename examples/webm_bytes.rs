//! Scratch: what does indexing and then reading every block cost the source, in
//! bytes? Cache state makes wall time lie on a mount; bytes and seeks do not.
use fvid::container::webm::{Limits, WebmReader};
use std::{
    cell::Cell,
    io::{BufReader, Read, Seek, SeekFrom},
    rc::Rc,
    time::Instant,
};
#[derive(Clone)]
struct Count {
    bytes: Rc<Cell<u64>>,
    seeks: Rc<Cell<u64>>,
}
struct Log<R> {
    inner: R,
    count: Count,
}
impl<R: Read> Read for Log<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.bytes.set(self.count.bytes.get() + n as u64);
        Ok(n)
    }
}
impl<R: Seek> Seek for Log<R> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.count.seeks.set(self.count.seeks.get() + 1);
        self.inner.seek(from)
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("expected WebM path")?;
    let kib: usize = std::env::args().nth(2).unwrap_or("1024".into()).parse()?;
    let limit: usize = std::env::args().nth(3).unwrap_or("900".into()).parse()?;
    let count = Count {
        bytes: Rc::new(Cell::new(0)),
        seeks: Rc::new(Cell::new(0)),
    };
    let start = Instant::now();
    let mut r = WebmReader::open(
        BufReader::with_capacity(
            kib << 10,
            Log {
                inner: std::fs::File::open(&path)?,
                count: count.clone(),
            },
        ),
        Limits::default(),
    )?;
    let opened = start.elapsed();
    let (bytes, seeks) = (count.bytes.get(), count.seeks.get());
    println!(
        "{kib} KiB buffer: open indexed {} packets in {opened:?}, {bytes} source bytes read in {seeks} seeks",
        r.packets.len(),
    );
    let blocks = r.packets.iter().take(limit).map(|p| p.size as u64).sum::<u64>();
    let before = (count.bytes.get(), count.seeks.get());
    let start = Instant::now();
    for i in 0..limit.min(r.packets.len()) {
        let _ = r.read_packet(i)?;
    }
    println!(
        "then read {limit} blocks of {blocks} bytes in {:?}: {} more source bytes, {} more seeks",
        start.elapsed(),
        count.bytes.get() - before.0,
        count.seeks.get() - before.1,
    );
    Ok(())
}
