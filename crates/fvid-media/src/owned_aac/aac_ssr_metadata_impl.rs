#[derive(Clone)]
pub(crate) struct PacketQueue<T> {
    entries: [Option<(i64, T)>; 2],
    head: usize,
    len: usize,
}
impl<T> Default for PacketQueue<T> {
    fn default() -> Self {
        Self {
            entries: [None, None],
            head: 0,
            len: 0,
        }
    }
}
impl<T> PacketQueue<T> {
    pub(crate) fn push(&mut self, stamp: i64, value: T) -> Result<()> {
        if self.len == 2 {
            return Err(invalid("SSR metadata exceeds one packet lookahead"));
        }
        self.entries[(self.head + self.len) % 2] = Some((stamp, value));
        self.len += 1;
        Ok(())
    }
    pub(crate) fn take(&mut self, stamp: i64) -> Result<T> {
        let next = self.entries[self.head]
            .as_ref()
            .ok_or_else(|| invalid("SSR output has no packet metadata"))?;
        if next.0 != stamp {
            return Err(invalid("SSR output packet metadata stamp mismatch"));
        }
        let (_, value) = self.entries[self.head].take().unwrap();
        self.head = (self.head + 1) % 2;
        self.len -= 1;
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_metadata_is_bounded_atomic_and_checkpointable() {
        let mut q = PacketQueue::default();
        q.push(-1024, 17).unwrap();
        q.push(0, 31).unwrap();
        let checkpoint = q.clone();
        assert!(q.push(1024, 47).is_err());
        assert!(q.take(0).is_err());
        assert_eq!(q.take(-1024).unwrap(), 17);
        assert_eq!(q.take(0).unwrap(), 31);
        assert!(q.take(1024).is_err());
        q = checkpoint;
        assert_eq!(q.take(-1024).unwrap(), 17);
        q.push(1024, 47).unwrap();
        assert_eq!(q.take(0).unwrap(), 31);
        assert_eq!(q.take(1024).unwrap(), 47);
    }
}
