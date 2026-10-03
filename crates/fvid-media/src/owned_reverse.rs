//! Reverse video through a private fixed-record spool, with one replay buffer.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, String>;
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Reverse {
    file: Option<File>,
    path: PathBuf,
    count: u64,
    length: Option<usize>,
    failed: bool,
    finished: bool,
}
impl Reverse {
    pub(crate) fn new() -> Result<Self> {
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-reverse-{}-{}.spool",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file: Some(file),
                        path,
                        count: 0,
                        length: None,
                        failed: false,
                        finished: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("reverse temporary storage: {e}")),
            }
        }
    }
    fn stride(&self) -> Result<u64> {
        u64::try_from(self.length.unwrap_or(0))
            .ok()
            .and_then(|n| n.checked_add(16))
            .ok_or("reverse record size overflow".into())
    }
    pub(crate) fn push(&mut self, data: &[u8], pts: u64, duration: u64) -> Result<()> {
        if self.failed || self.finished {
            return Err("reverse spool is not writable".into());
        }
        if self.length.is_some_and(|n| n != data.len()) {
            return Err("reverse frame storage changed".into());
        }
        self.length = Some(data.len());
        let next = self
            .count
            .checked_add(1)
            .ok_or("reverse frame count overflow")?;
        next.checked_mul(self.stride()?)
            .ok_or("reverse spool size overflow")?;
        let mut timing = [0u8; 16];
        timing[..8].copy_from_slice(&pts.to_le_bytes());
        timing[8..].copy_from_slice(&duration.to_le_bytes());
        let result = self
            .file
            .as_mut()
            .unwrap()
            .write_all(&timing)
            .and_then(|_| self.file.as_mut().unwrap().write_all(data));
        if let Err(e) = result {
            self.failed = true;
            return Err(e.to_string());
        }
        self.count = next;
        Ok(())
    }
    pub(crate) fn flush(
        &mut self,
        emit: &mut impl FnMut(&[u8], u64, u64) -> Result<()>,
    ) -> Result<u64> {
        if self.failed {
            return Err("reverse spool failed".into());
        }
        if self.finished {
            return Ok(0);
        }
        let result = self.replay(emit);
        if result.is_err() {
            self.failed = true;
        } else {
            self.finished = true;
        }
        result
    }
    fn replay(&mut self, emit: &mut impl FnMut(&[u8], u64, u64) -> Result<()>) -> Result<u64> {
        let stride = self.stride()?;
        let mut data = Vec::new();
        let length = self.length.unwrap_or(0);
        data.try_reserve_exact(length).map_err(|e| e.to_string())?;
        data.resize(length, 0);
        let file = self.file.as_mut().unwrap();
        for position in 0..self.count {
            file.seek(SeekFrom::Start(position * stride))
                .map_err(|e| e.to_string())?;
            let mut timing = [0u8; 16];
            file.read_exact(&mut timing).map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Start((self.count - position - 1) * stride + 16))
                .map_err(|e| e.to_string())?;
            file.read_exact(&mut data).map_err(|e| e.to_string())?;
            emit(
                &data,
                u64::from_le_bytes(timing[..8].try_into().unwrap()),
                u64::from_le_bytes(timing[8..].try_into().unwrap()),
            )?;
        }
        Ok(self.count)
    }
}
impl Drop for Reverse {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn reverses_payloads_but_keeps_forward_timing_and_cleans_storage() {
        use super::Reverse;
        let mut reverse = Reverse::new().unwrap();
        let path = reverse.path.clone();
        for (value, pts, duration) in [(0, 0, 10), (1, 10, 15), (2, 25, 17)] {
            reverse.push(&[value], pts, duration).unwrap();
        }
        let mut seen = Vec::new();
        assert_eq!(
            reverse
                .flush(&mut |data, pts, duration| {
                    seen.push((data[0], pts, duration));
                    Ok(())
                })
                .unwrap(),
            3
        );
        assert_eq!(seen, [(2, 0, 10), (1, 10, 15), (0, 25, 17)]);
        assert!(reverse.push(&[0], 0, 0).is_err());
        drop(reverse);
        assert!(!path.exists());
    }
    #[test]
    fn replay_callback_failure_is_terminal_and_removes_the_spool() {
        use super::Reverse;
        let mut reverse = Reverse::new().unwrap();
        let path = reverse.path.clone();
        reverse.push(&[1], 0, 1).unwrap();
        assert_eq!(
            reverse
                .flush(&mut |_, _, _| Err("stop".into()))
                .unwrap_err(),
            "stop"
        );
        assert!(reverse.flush(&mut |_, _, _| Ok(())).is_err());
        drop(reverse);
        assert!(!path.exists());
    }
}
