//! Incremental native WavPack file reading, including optional correction files.
//! Headers build a seek index without reading skipped entropy bodies. Only the
//! requested sample frame is materialized; decoding and both CRCs stay owned.
use crate::audio::{AudioDecode, AudioPacket, AudioStream, AudioTrack, EncodedPacket};
use crate::codec::wavpack_decoder::{FrameInfo, WavpackDecoder};
use crate::{Result, invalid};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;
pub const TAG: &str = "wavpack";
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub file_bytes: u64,
    pub packet_bytes: usize,
    pub index_entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: u64::MAX,
            packet_bytes: 64 << 20,
            index_entries: usize::MAX,
        }
    }
}
trait Source: Read + Seek + Send {}
impl<T: Read + Seek + Send> Source for T {}
#[derive(Clone, Debug)]
struct Group {
    offset: u64,
    length: usize,
    index: u64,
    samples: u32,
    channels: u16,
    total: Option<u64>,
    headers: Vec<[u8; 20]>,
}
struct Input {
    reader: Box<dyn Source>,
    at: u64,
    end: u64,
    limits: Limits,
    seen: bool,
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn exact(reader: &mut dyn Source, at: u64, bytes: &mut [u8]) -> Result<()> {
    reader.seek(SeekFrom::Start(at))?;
    reader.read_exact(bytes)?;
    Ok(())
}
impl Input {
    fn new<R: Read + Seek + Send + 'static>(reader: R, limits: Limits) -> Result<Self> {
        let mut reader: Box<dyn Source> = Box::new(reader);
        let end = reader.seek(SeekFrom::End(0))?;
        if end > limits.file_bytes {
            return Err(invalid("WavPack file exceeds input budget"));
        }
        let mut at = 0;
        if end >= 10 {
            let mut head = [0; 10];
            exact(&mut *reader, 0, &mut head)?;
            if &head[..3] == b"ID3" {
                if !matches!(head[3], 2..=4)
                    || head[4] == 255
                    || head[6..10].iter().any(|&v| v > 127)
                {
                    return Err(invalid("invalid WavPack ID3 prefix"));
                }
                let size = head[6..10]
                    .iter()
                    .fold(0u64, |n, &v| (n << 7) | u64::from(v));
                at = 10 + size;
                if head[3] == 4 && head[5] & 16 != 0 {
                    let mut footer = [0; 10];
                    if at.checked_add(10).is_none_or(|v| v > end) {
                        return Err(invalid("truncated WavPack ID3 footer"));
                    }
                    exact(&mut *reader, at, &mut footer)?;
                    if &footer[..3] != b"3DI" || footer[3..] != head[3..] {
                        return Err(invalid("invalid WavPack ID3 footer"));
                    }
                    at += 10;
                }
                if at > end {
                    return Err(invalid("WavPack ID3 prefix exceeds file"));
                }
            }
        }
        Ok(Self {
            reader,
            at,
            end,
            limits,
            seen: false,
        })
    }
    fn tags(&mut self) -> Result<bool> {
        let at = self.at;
        let mut tag_end = self.end;
        if tag_end >= 128 {
            let mut id = [0; 3];
            exact(&mut *self.reader, tag_end - 128, &mut id)?;
            if &id == b"TAG" {
                tag_end -= 128;
            }
        }
        if tag_end == at {
            self.at = self.end;
            return Ok(true);
        }
        if tag_end >= 32 {
            let mut footer = [0; 32];
            exact(&mut *self.reader, tag_end - 32, &mut footer)?;
            if &footer[..8] == b"APETAGEX" {
                let version = u32le(&footer, 8);
                let size = u64::from(u32le(&footer, 12));
                let flags = u32le(&footer, 20);
                if !matches!(version, 1000 | 2000) || size < 32 || flags & 0x60000000 != 0 {
                    return Err(invalid("invalid WavPack APE footer"));
                }
                let mut start = tag_end
                    .checked_sub(size)
                    .ok_or_else(|| invalid("WavPack APE size exceeds file"))?;
                if flags & 0x80000000 != 0 {
                    start = start
                        .checked_sub(32)
                        .ok_or_else(|| invalid("WavPack APE header exceeds file"))?;
                    let mut header = [0; 32];
                    exact(&mut *self.reader, start, &mut header)?;
                    if &header[..8] != b"APETAGEX"
                        || header[8..20] != footer[8..20]
                        || u32le(&header, 20) != (flags | 0x20000000)
                    {
                        return Err(invalid("WavPack APE header/footer mismatch"));
                    }
                }
                if start == at {
                    self.at = self.end;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn group(&mut self) -> Result<Option<Group>> {
        let mut group: Option<Group> = None;
        while self.at < self.end {
            if self.end - self.at < 32 {
                return Err(invalid("truncated WavPack native block header"));
            }
            let mut h = [0; 32];
            exact(&mut *self.reader, self.at, &mut h)?;
            if &h[..4] != b"wvpk" {
                if group.is_none() && self.seen && self.tags()? {
                    return Ok(None);
                }
                return Err(invalid("invalid WavPack native block tag"));
            }
            let size = u32le(&h, 4);
            let length = usize::try_from(u64::from(size) + 8)
                .map_err(|_| invalid("WavPack block size overflow"))?;
            if size == u32::MAX || length < 32 {
                return Err(invalid("invalid WavPack native block length"));
            }
            let end = self
                .at
                .checked_add(length as u64)
                .filter(|&v| v <= self.end)
                .ok_or_else(|| invalid("WavPack native block exceeds file"))?;
            let version = u16::from_le_bytes([h[8], h[9]]);
            if !(0x402..=0x410).contains(&version) {
                return Err(invalid("unsupported WavPack native version"));
            }
            let count = u32le(&h, 20);
            if count == 0 {
                if group.is_some() {
                    return Err(invalid("WavPack metadata block interrupts sample frame"));
                }
                if length > self.limits.packet_bytes {
                    return Err(invalid("WavPack metadata block exceeds packet budget"));
                }
                self.at = end;
                continue;
            }
            if count > crate::codec::wavpack_decoder::MAX_BLOCK_SAMPLES {
                return Err(invalid("WavPack native unusable block sample count"));
            }
            let flags = u32le(&h, 24);
            let channels = if flags & 4 != 0 { 1 } else { 2 };
            let index = u64::from(u32le(&h, 16)) | (u64::from(h[10]) << 32);
            let total = (u32le(&h, 12) != u32::MAX)
                .then(|| u64::from(u32le(&h, 12)) + (u64::from(h[11]) << 32) - u64::from(h[11]));
            if (flags & 0x800 != 0) != group.is_none() {
                return Err(invalid("WavPack native initial component mismatch"));
            }
            if flags & 4 != 0 && flags & (1 << 30) != 0 {
                return Err(invalid("WavPack native conflicting mono flags"));
            }
            let g = group.get_or_insert_with(|| Group {
                offset: self.at,
                length: 0,
                index,
                samples: count,
                channels: 0,
                total,
                headers: Vec::new(),
            });
            if g.index != index
                || g.samples != count
                || g.total.zip(total).is_some_and(|(a, b)| a != b)
            {
                return Err(invalid("WavPack native component window mismatch"));
            }
            if let Some(first) = g.headers.first() {
                if first[0..2] != h[8..10] || u32le(first, 16) & (0x80000080) != flags & 0x80000080
                {
                    return Err(invalid("WavPack native component format mismatch"));
                }
            }
            g.length = g
                .length
                .checked_add(length)
                .filter(|&v| v <= self.limits.packet_bytes)
                .ok_or_else(|| invalid("WavPack frame exceeds packet budget"))?;
            g.channels = g
                .channels
                .checked_add(channels)
                .filter(|&n| n <= 4096)
                .ok_or_else(|| invalid("WavPack native channel count exceeds format"))?;
            let mut signature = [0; 20];
            signature.copy_from_slice(&h[8..28]);
            g.headers
                .try_reserve(1)
                .map_err(|_| invalid("cannot allocate WavPack component headers"))?;
            g.headers.push(signature);
            self.at = end;
            if flags & 0x1000 != 0 {
                self.seen = true;
                return Ok(group);
            }
        }
        if group.is_some() {
            return Err(invalid("WavPack native sample frame lacks final component"));
        }
        Ok(None)
    }
    fn payload(&mut self, g: &Group) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        data.try_reserve_exact(g.length)
            .map_err(|_| invalid("cannot allocate WavPack native packet"))?;
        data.resize(g.length, 0);
        exact(&mut *self.reader, g.offset, &mut data)?;
        Ok(data)
    }
}
fn pair(main: &Group, correction: &Group) -> Result<()> {
    if main.headers != correction.headers {
        return Err(invalid(
            "WavPack native correction headers disagree with main",
        ));
    }
    Ok(())
}
struct Entry {
    main: Group,
    correction: Option<Group>,
    pts: u64,
}
pub struct WavpackAudioReader {
    main: Input,
    correction: Option<Input>,
    limits: Limits,
    frames: Vec<Entry>,
    cursor: usize,
    info: FrameInfo,
    origin: u64,
    end_sample: u64,
    total: Option<u64>,
    done: bool,
    floor: Option<u64>,
    error: Option<String>,
    mask: Option<u32>,
}
impl WavpackAudioReader {
    pub fn open<R: Read + Seek + Send + 'static>(reader: R, limits: Limits) -> Result<Self> {
        let mut main = Input::new(reader, limits)?;
        let first = main
            .group()?
            .ok_or_else(|| invalid("WavPack file contains no sample frames"))?;
        if limits.index_entries == 0 {
            return Err(invalid("WavPack index exceeds entry budget"));
        }
        let bytes = main.payload(&first)?;
        let info = WavpackDecoder::inspect_frame(&bytes, first.channels)?;
        let end_sample = first
            .index
            .checked_add(u64::from(first.samples))
            .ok_or_else(|| invalid("WavPack native sample endpoint overflow"))?;
        if first.total.is_some_and(|n| n < end_sample) {
            return Err(invalid("WavPack total samples precede first frame end"));
        }
        let mask = info.speaker_mask.or(match info.channels {
            1 => Some(4),
            2 => Some(3),
            _ => None,
        });
        let origin = first.index;
        let total = first.total;
        let mut frames = Vec::new();
        frames
            .try_reserve(1)
            .map_err(|_| invalid("cannot allocate WavPack native index"))?;
        frames.push(Entry {
            main: first,
            correction: None,
            pts: 0,
        });
        Ok(Self {
            main,
            correction: None,
            limits,
            frames,
            cursor: 0,
            info,
            origin,
            end_sample,
            total,
            done: false,
            floor: None,
            error: None,
            mask,
        })
    }
    pub fn with_correction<R: Read + Seek + Send + 'static, C: Read + Seek + Send + 'static>(
        reader: R,
        correction: C,
        limits: Limits,
    ) -> Result<Self> {
        Self::open(reader, limits)?.attach_correction(correction)
    }
    pub fn attach_correction<C: Read + Seek + Send + 'static>(mut self, reader: C) -> Result<Self> {
        if self.cursor != 0 || self.correction.is_some() {
            return Err(invalid("WavPack correction must attach before playback"));
        }
        let mut correction = Input::new(reader, self.limits)?;
        for entry in &mut self.frames {
            let c = correction
                .group()?
                .ok_or_else(|| invalid("WavPack correction ends before main stream"))?;
            pair(&entry.main, &c)?;
            if entry
                .main
                .length
                .checked_add(c.length)
                .is_none_or(|n| n > self.limits.packet_bytes)
            {
                return Err(invalid("WavPack paired frame exceeds packet budget"));
            }
            entry.correction = Some(c);
        }
        self.correction = Some(correction);
        Ok(self)
    }
    pub fn indexed_frames(&self) -> usize {
        self.frames.len()
    }
    pub fn fully_indexed(&self) -> bool {
        self.done
    }
    fn scan(&mut self) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        let next = self.main.group()?;
        let Some(main) = next else {
            if let Some(c) = &mut self.correction {
                if c.group()?.is_some() {
                    return Err(invalid("WavPack correction contains extra frames"));
                }
            }
            if self.total.is_some_and(|n| n != self.end_sample) {
                return Err(invalid("WavPack native stream ends before declared total"));
            }
            self.done = true;
            return Ok(false);
        };
        if self.frames.len() >= self.limits.index_entries {
            return Err(invalid("WavPack index exceeds entry budget"));
        }
        if main.index != self.end_sample {
            return Err(invalid("WavPack native sample indices are not contiguous"));
        }
        if main.channels != self.info.channels
            || main.headers[0][0..2] != self.frames[0].main.headers[0][0..2]
            || u32le(&main.headers[0], 16) & 0x80000080
                != u32le(&self.frames[0].main.headers[0], 16) & 0x80000080
        {
            return Err(invalid("WavPack native format changes between frames"));
        }
        if let Some(total) = main.total {
            if self.total.is_some_and(|old| old != total) {
                return Err(invalid("WavPack native total sample count changed"));
            }
            self.total = Some(total);
        }
        let end = main
            .index
            .checked_add(u64::from(main.samples))
            .ok_or_else(|| invalid("WavPack native sample endpoint overflow"))?;
        if self.total.is_some_and(|total| end > total) {
            return Err(invalid("WavPack frame extends beyond declared total"));
        }
        let correction = if let Some(c) = &mut self.correction {
            let extra = c
                .group()?
                .ok_or_else(|| invalid("WavPack correction ends before main stream"))?;
            pair(&main, &extra)?;
            if main
                .length
                .checked_add(extra.length)
                .is_none_or(|n| n > self.limits.packet_bytes)
            {
                return Err(invalid("WavPack paired frame exceeds packet budget"));
            }
            Some(extra)
        } else {
            None
        };
        self.frames
            .try_reserve(1)
            .map_err(|_| invalid("cannot allocate WavPack native index"))?;
        let pts = main.index - self.origin;
        self.frames.push(Entry {
            main,
            correction,
            pts,
        });
        self.end_sample = end;
        Ok(true)
    }
    fn next(&mut self) -> Result<Option<EncodedPacket>> {
        if let Some(error) = &self.error {
            return Err(invalid(error));
        }
        if self.cursor == self.frames.len() && !self.scan()? {
            return Ok(None);
        }
        let e = &self.frames[self.cursor];
        let mut data = self.main.payload(&e.main)?;
        let info = WavpackDecoder::inspect_frame(&data, self.info.channels)?;
        if info.sample_rate != self.info.sample_rate
            || info.bits_per_sample != self.info.bits_per_sample
            || info.dsd != self.info.dsd
            || info.float != self.info.float
            || info
                .speaker_mask
                .zip(self.mask)
                .is_some_and(|(a, b)| a != b)
        {
            return Err(invalid("WavPack native frame geometry changed"));
        }
        if let (Some(source), Some(g)) = (&mut self.correction, &e.correction) {
            let c = source.payload(g)?;
            data = WavpackDecoder::native_with_correction(&data, &c, self.info.channels)?;
        }
        let packet = EncodedPacket {
            data,
            pts: e.pts as i64,
            duration: i64::from(e.main.samples),
        };
        self.cursor += 1;
        Ok(Some(packet))
    }
}
impl AudioStream for WavpackAudioReader {
    fn codec(&self) -> &str {
        TAG
    }
    fn timescale(&self) -> u32 {
        self.info.sample_rate
    }
    fn sample_rate(&self) -> u32 {
        self.info.sample_rate
    }
    fn channels(&self) -> u16 {
        self.info.channels
    }
    fn bits_per_sample(&self) -> u16 {
        self.info.bits_per_sample
    }
    fn extra_data(&self) -> &[u8] {
        &[]
    }
    fn speaker_mask(&self) -> Result<Option<u32>> {
        Ok(self.mask)
    }
    fn make_decoder(&self) -> Result<Box<dyn AudioDecode>> {
        Ok(Box::new(
            WavpackDecoder::new(self.info.channels)?.with_speaker_mask(self.mask)?,
        ))
    }
    fn duration(&self) -> Option<Duration> {
        self.total
            .or(self.done.then_some(self.end_sample))
            .and_then(|end| end.checked_sub(self.origin))
            .map(|n| crate::audio::sample_duration(n, self.info.sample_rate))
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.info.sample_rate,
            channels: self.info.channels,
            name: String::new(),
            language: String::new(),
        }]
    }
    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let result = self.next();
        if let Err(e) = &result {
            self.error = Some(e.to_string());
        }
        result
    }
    fn validate_eof(&self) -> Result<()> {
        if let Some(e) = &self.error {
            Err(invalid(e))
        } else {
            Ok(())
        }
    }
    fn rewind(&mut self) {
        self.cursor = 0;
        self.floor = None;
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        let mut target = pts.max(0) as u64;
        while !self.done && self.end_sample.saturating_sub(self.origin) <= target {
            match self.scan() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return target as i64;
                }
            }
        }
        if self.done {
            target = target.min(self.end_sample - self.origin);
        }
        self.floor = Some(target);
        self.cursor = if self.done && target == self.end_sample - self.origin {
            self.frames.len()
        } else if self.info.dsd {
            0
        } else {
            self.frames
                .partition_point(|e| e.pts <= target)
                .saturating_sub(1)
        };
        target as i64
    }
    fn present_decoded(
        &self,
        mut packet: AudioPacket,
        source_pts: i64,
    ) -> Result<Option<AudioPacket>> {
        if source_pts < 0 {
            return Err(invalid("negative WavPack native PCM timestamp"));
        }
        let stride = usize::from(self.info.channels) * 4;
        if packet.data.len() % stride != 0 {
            return Err(invalid("WavPack native PCM has incomplete channel frame"));
        }
        let samples = packet.data.len() / stride;
        let first = self
            .floor
            .unwrap_or(0)
            .saturating_sub(source_pts as u64)
            .min(samples as u64) as usize;
        if first == samples {
            return Ok(None);
        }
        if first != 0 {
            packet.data.drain(..first * stride);
        }
        packet.pts = source_pts as u64 + first as u64;
        packet.timebase_num = 1;
        packet.timebase_den = self.info.sample_rate;
        Ok(Some(packet))
    }
}
