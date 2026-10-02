//! Retained allocation payload accounting; no allocation during inspection.
use std::{mem::size_of, sync::Arc};
// Two owners (decoder + checkpoint), u8 channel count, 16 coupling tags,
// four windows and two IMDCT tables per synthesis state.
const MAX_SHARED: usize = 2 * (u8::MAX as usize + 16) * 6;
pub(crate) struct Footprint {
    seen: [usize; MAX_SHARED],
    used: usize,
    bytes: usize,
}
impl Footprint {
    pub(crate) fn new() -> Self {
        Self {
            seen: [0; MAX_SHARED],
            used: 0,
            bytes: 0,
        }
    }
    pub(crate) fn add(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("AAC allocation accounting overflow")?;
        Ok(())
    }
    pub(crate) fn vector<T>(&mut self, vector: &Vec<T>) -> Result<(), String> {
        self.add(
            vector
                .capacity()
                .checked_mul(size_of::<T>())
                .ok_or("AAC allocation accounting overflow")?,
        )
    }
    pub(crate) fn shared<T>(&mut self, value: &Arc<T>) -> Result<bool, String> {
        let address = Arc::as_ptr(value) as usize;
        if self.seen[..self.used].contains(&address) {
            return Ok(false);
        }
        if self.used == self.seen.len() {
            return Err("AAC shared allocation accounting capacity exceeded".into());
        }
        self.seen[self.used] = address;
        self.used += 1;
        self.add(size_of::<T>())?;
        Ok(true)
    }
    pub(crate) fn total(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spare_capacity_and_shared_owners_are_counted_once() {
        let storage = Vec::<u32>::with_capacity(17);
        let capacity = storage.capacity();
        let shared = Arc::new(storage);
        let alias = shared.clone();
        let mut footprint = Footprint::new();
        assert!(footprint.shared(&shared).unwrap());
        footprint.vector(&shared).unwrap();
        assert!(!footprint.shared(&alias).unwrap());
        assert_eq!(
            footprint.total(),
            size_of::<Vec<u32>>() + capacity * size_of::<u32>()
        );
    }
}
