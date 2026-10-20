use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub struct SeekFault<'a> {
    source: Cursor<&'a [u8]>,
    pub armed: Arc<AtomicBool>,
    pub fired: Arc<AtomicBool>,
    kind: io::ErrorKind,
}
impl<'a> SeekFault<'a> {
    pub fn new(bytes: &'a [u8], kind: io::ErrorKind) -> Self {
        Self {
            source: Cursor::new(bytes),
            armed: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
            kind,
        }
    }
}
impl Read for SeekFault<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.source.read(out)
    }
}
impl Seek for SeekFault<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = self.source.seek(from)?;
        if matches!(from, SeekFrom::Start(_)) && self.armed.swap(false, Ordering::Relaxed) {
            self.fired.store(true, Ordering::Relaxed);
            Err(io::Error::new(
                self.kind,
                "synthetic seek moved before failing",
            ))
        } else {
            Ok(position)
        }
    }
}
