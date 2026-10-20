//! Bounded VP9 decode checkpoints. Picture buffers remain immutable and shared.
use crate::codec::vp9_decoder::Decoder;
use std::collections::{BTreeMap, HashMap};
use crate::playback_native::{RawFrame, Planar8, PackedPlanar};
use std::sync::Arc;
#[derive(Clone)]
enum Image { Planes(Arc<Planar8>), Packed(Arc<PackedPlanar>) }
#[derive(Clone)]
pub(super) struct ImageFrame {
    pub start: u128, pub end: u128, pub dimensions: [usize; 2],
    pub next: Option<u128>, pub eof: bool,
    image: Image, bytes: usize,
}
impl ImageFrame {
    pub fn raw(&self) -> RawFrame {
        match &self.image { Image::Planes(p) => RawFrame::Planar8(p.clone()), Image::Packed(p) => RawFrame::Planar(p.clone()) }
    }
}


pub(super) struct Point {
    pub index: usize,
    pub pts: i64,
    pub decoder: Decoder,
    storage: Vec<(usize, usize)>,
}
pub(super) struct Cache {
    points: BTreeMap<(i64, usize), Point>,
    frames: BTreeMap<u128, ImageFrame>,
    pictures: HashMap<usize, (usize, usize)>,
    bytes: usize,
    limit: usize,
    pub hits: usize,
    pub preparing: bool,
    coverage: Vec<(u128, u128)>,
    coverage_dirty: bool,
}
impl Cache {
    pub fn new(limit: usize) -> Self {
        Self { points: BTreeMap::new(), frames: BTreeMap::new(), pictures: HashMap::new(), bytes: 0, limit, hits: 0, preparing: false, coverage: Vec::new(), coverage_dirty: true }
    }
    pub fn enabled(&self) -> bool { self.limit > 0 }
    pub fn wants(&self, pts: i64, index: usize) -> bool {
        self.limit > 0 && self.points.range(..=(pts, index)).next_back()
            .is_none_or(|(_, point)| pts.saturating_sub(point.pts) >= 100_000_000)
    }
    pub fn insert(&mut self, pts: i64, index: usize, decoder: &Decoder) {
        if !self.wants(pts, index) { return; }
        let storage = decoder.retained_storage();
        let state_bytes = std::mem::size_of::<Decoder>() + 256;
        self.bytes = self.bytes.saturating_add(state_bytes);
        for &(key, bytes) in &storage {
            let entry = self.pictures.entry(key).or_insert_with(|| {
                self.bytes = self.bytes.saturating_add(bytes);
                (bytes, 0)
            });
            entry.1 += 1;
        }
        self.points.insert((pts, index), Point { index, pts, decoder: decoder.clone(), storage });
        self.trim();
    }
    fn trim(&mut self) {
        while self.bytes > self.limit {
            if let Some((_, point)) = self.points.pop_first() {
                self.bytes = self.bytes.saturating_sub(std::mem::size_of::<Decoder>() + 256);
                for (key, _) in point.storage {
                    let entry = self.pictures.get_mut(&key).unwrap();
                    entry.1 -= 1;
                    if entry.1 == 0 {
                        self.bytes = self.bytes.saturating_sub(entry.0);
                        self.pictures.remove(&key);
                    }
                }
            } else if let Some((_, frame)) = self.frames.pop_first() {
                self.coverage_dirty = true;
                self.bytes = self.bytes.saturating_sub(frame.bytes);
            } else { break; }
        }
    }
    pub fn insert_frame(&mut self, start: u128, end: u128, dimensions: [usize; 2], raw: &RawFrame, previous: Option<u128>) {
        if self.limit == 0 { return; }
        let (image, bytes) = match raw {
            RawFrame::Planar8(p) => (Image::Planes(p.clone()), p.y.capacity() + p.cb.capacity() + p.cr.capacity()),
            RawFrame::Planar(p) => (Image::Packed(p.clone()), p.frame.data.capacity() + p.alpha.as_ref().map_or(0, |a| a.capacity())),
            _ => return,
        };
        if let Some(previous) = previous.filter(|p| *p < start) {
            if let Some(frame) = self.frames.get_mut(&previous) { frame.next = Some(start); }
        }
        if self.frames.contains_key(&start) { return; }
        let bytes = bytes.saturating_add(256);
        if bytes > self.limit { return; }
        self.coverage_dirty = true;
        self.frames.insert(start, ImageFrame { start, end, dimensions, image, bytes, next: None, eof: false });
        self.bytes = self.bytes.saturating_add(bytes);
        self.trim();
    }
    pub fn finish_frames(&mut self, previous: Option<u128>) {
        if let Some(frame) = previous.and_then(|p| self.frames.get_mut(&p)) { frame.eof = true; }
    }
    pub fn frame_at(&mut self, target: u128) -> Option<ImageFrame> {
        let (_, frame) = self.frames.range(..=target).next_back()?;
        if target >= frame.end { return None; }
        self.hits += 1;
        Some(frame.clone())
    }
    pub fn next_frame(&mut self, previous: u128) -> (Option<ImageFrame>, bool) {
        let Some(frame) = self.frames.get(&previous) else { return (None, false); };
        let end = frame.eof;
        let next = frame.next.and_then(|p| self.frames.get(&p)).cloned();
        if next.is_some() { self.hits += 1; }
        (next, end)
    }
    fn coverage(&mut self) -> Vec<(u128, u128)> {
        if self.coverage_dirty {
            let mut ranges: Vec<(u128, u128)> = Vec::new();
            for frame in self.frames.values() {
                if let Some(last) = ranges.last_mut().filter(|last| last.1 >= frame.start) {
                    last.1 = last.1.max(frame.end);
                } else { ranges.push((frame.start, frame.end)); }
            }
            self.coverage = ranges;
            self.coverage_dirty = false;
        }
        self.coverage.clone()
    }
    pub fn frames_complete(&self) -> bool {
        let Some((&first, _)) = self.frames.first_key_value() else { return false; };
        if first != 0 { return false; }
        let mut at = first;
        loop {
            let Some(frame) = self.frames.get(&at) else { return false; };
            if frame.eof { return true; }
            let Some(next) = frame.next.filter(|next| *next > at) else { return false; };
            at = next;
        }
    }
    pub fn frame_count(&self) -> usize { self.frames.len() }
    pub fn restore(&mut self, target: i64, first_index: usize) -> Option<(usize, i64, Decoder)> {
        let (_, point) = self.points.range(..=(target, usize::MAX)).next_back()?;
        if point.index < first_index { return None; }
        self.hits += 1;
        Some((point.index, point.pts, point.decoder.clone()))
    }
    pub fn stats(&self) -> (usize, usize, usize) { (self.points.len(), self.bytes, self.hits) }
}

/// Metadata view only: observing preparation never keeps a closed item alive.
#[derive(Clone)]
pub struct Status(std::sync::Weak<std::sync::Mutex<Cache>>);
impl Status {
    pub(super) fn new(cache: &Arc<std::sync::Mutex<Cache>>) -> Self { Self(Arc::downgrade(cache)) }
    pub fn is_preparing(&self) -> bool {
        self.0.upgrade().is_some_and(|cache| cache.lock().unwrap_or_else(|p| p.into_inner()).preparing)
    }
    pub fn ranges(&self, duration: std::time::Duration) -> Vec<[f32; 2]> {
        if duration.is_zero() { return Vec::new(); }
        let Some(cache) = self.0.upgrade() else { return Vec::new(); };
        let total = duration.as_nanos() as f64;
        cache.lock().unwrap_or_else(|p| p.into_inner()).coverage().into_iter()
            .map(|(a,b)| [(a as f64 / total).clamp(0.0,1.0) as f32, (b as f64 / total).clamp(0.0,1.0) as f32]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eviction_respects_budget_and_never_restores_a_point_before_the_gop() {
        let state = std::mem::size_of::<Decoder>() + 256;
        let decoder = Decoder::new(1 << 20);
        let mut cache = Cache::new(state * 2);
        for index in 0..4 { cache.insert(index as i64 * 100_000_000, index, &decoder); }
        assert_eq!(cache.stats().0, 2);
        assert!(cache.stats().1 <= state * 2);
        assert!(cache.restore(100_000_000, 0).is_none());
        assert!(cache.restore(300_000_000, 4).is_none());
        assert_eq!(cache.restore(300_000_000, 0).unwrap().0, 3);
    }
    #[test]
    fn status_view_does_not_keep_a_closed_items_cache_alive() {
        let cache = Arc::new(std::sync::Mutex::new(Cache::new(1024)));
        let status = Status::new(&cache);
        cache.lock().unwrap().preparing = true;
        assert!(status.is_preparing());
        assert!(status.ranges(std::time::Duration::from_secs(1)).is_empty());
        drop(cache);
        assert!(!status.is_preparing());
        assert!(status.ranges(std::time::Duration::from_secs(1)).is_empty());
    }
    #[test]
    fn zero_budget_keeps_no_decoder_or_picture_storage() {
        let mut cache = Cache::new(0);
        cache.insert(0, 0, &Decoder::new(1 << 20));
        assert_eq!(cache.stats(), (0, 0, 0));
    }
}
