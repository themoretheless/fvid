//! Deterministic reader faults at an exact byte offset, including short reads.
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct FaultReader {
    inner: Cursor<&'static [u8]>,
    fault_at: u64,
    kind: io::ErrorKind,
    pub fired: Arc<AtomicBool>,
    max_read: usize,
}
impl FaultReader {
    pub fn new(data: &'static [u8], fault_at: u64, kind: io::ErrorKind, max_read: usize) -> Self {
        assert!(max_read > 0);
        Self {
            inner: Cursor::new(data),
            fault_at,
            kind,
            fired: Arc::new(AtomicBool::new(false)),
            max_read,
        }
    }
}
impl Read for FaultReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let position = self.inner.position();
        if position == self.fault_at && !self.fired.swap(true, Ordering::Relaxed) {
            return Err(io::Error::new(self.kind, "synthetic container I/O fault"));
        }
        let mut count = buf.len().min(self.max_read);
        if position < self.fault_at && !self.fired.load(Ordering::Relaxed) {
            count = count.min((self.fault_at - position) as usize);
        }
        self.inner.read(&mut buf[..count])
    }
}
impl Seek for FaultReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
