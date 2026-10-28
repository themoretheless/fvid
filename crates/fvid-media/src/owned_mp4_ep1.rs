//! Exact self-contained MP4 EP1 elementary-stream groups. Sample bytes are bare
//! AUs under SL predefined configuration 2; padb/fragment flags supply bit ends.
//! Presentation edits and player navigation are owned by the caller.
use crate::{
    owned_aac::{self, aac_ep1},
    owned_codec_config::aac_stream_descriptor,
    owned_mp4::{Mp4Reader, Track},
};
use std::io::{Read, Seek};
type Result<T> = owned_aac::Result<T>;
fn invalid(message: &str) -> owned_aac::Error {
    owned_aac::Error(message.into())
}
fn key(pts: i64, scale: u32) -> (i64, u32) {
    let (mut a, mut b) = (pts.unsigned_abs(), u64::from(scale));
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    (pts / a as i64, scale / a as u32)
}
struct Stream {
    index: usize,
    id: u32,
    es_id: u16,
    parent: Option<u16>,
    timescale: u32,
    configuration: Vec<u8>,
    samples: Vec<((i64, u32), usize)>,
}
pub struct Group {
    root: usize,
    streams: Vec<Stream>,
}
pub struct AccessUnit {
    pub stream_id: u16,
    pub depends_on: Option<u16>,
    pub data: Vec<u8>,
    pub bit_len: usize,
    pub pts: i64,
    pub duration: u64,
    pub timescale: u32,
}
impl AccessUnit {
    pub fn borrowed(&self) -> aac_ep1::AccessUnit<'_> {
        aac_ep1::AccessUnit {
            stream_id: self.stream_id,
            depends_on: self.depends_on,
            data: &self.data,
            bit_len: self.bit_len,
            pts: self.pts,
            duration: self.duration,
            timescale: self.timescale,
        }
    }
}
impl Group {
    /// Select a root ES explicitly. Unrelated audio tracks are not dependency
    /// members. A fork, repeated ES ID or ambiguous AU timestamp is an error.
    pub fn new(tracks: &[Track], root: usize) -> Result<Self> {
        let track = tracks
            .get(root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        if track.handler != *b"soun" || track.codec != *b"mp4a" {
            return Err(invalid("EP1 MP4 root is not MPEG-4 audio"));
        }
        let descriptor = aac_stream_descriptor(&track.configuration)?;
        if descriptor.depends_on.is_some() {
            return Err(invalid("EP1 MP4 selected track is not a root ES"));
        }
        if !owned_aac::config::AudioSpecificConfig::parse(descriptor.asc)?.separate_esc {
            return Err(invalid("EP1 MP4 root does not declare separate ESCs"));
        }
        let asc = descriptor.asc;
        let mut streams = Vec::new();
        let mut at = root;
        loop {
            let track = &tracks[at];
            track
                .ensure_constant_description()
                .map_err(|e| invalid(&e.to_string()))?;
            let d = aac_stream_descriptor(&track.configuration)?;
            if d.url.is_some() {
                return Err(invalid(
                    "EP1 MP4 external ES URL requires an external source",
                ));
            }
            if d.sl_config != Some(&[2][..]) {
                return Err(invalid("EP1 MP4 requires SL predefined configuration 2"));
            }
            if d.asc != asc {
                return Err(invalid("EP1 MP4 dependent ASC disagrees with root"));
            }
            if track.timescale == 0 {
                return Err(invalid("EP1 MP4 zero source timescale"));
            }
            if streams.iter().any(|s: &Stream| s.es_id == d.es_id) {
                return Err(invalid("EP1 MP4 repeated or cyclic ES ID"));
            }
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(track.samples.len())
                .map_err(|_| invalid("EP1 MP4 timing index allocation failed"))?;
            for i in 0..track.samples.len() {
                let sample = track
                    .samples
                    .get(i)
                    .ok_or_else(|| invalid("EP1 MP4 sample index out of range"))?;
                samples.push((key(sample.pts, track.timescale), i));
            }
            samples.sort_unstable_by_key(|row| row.0);
            if samples.windows(2).any(|rows| rows[0].0 == rows[1].0) {
                return Err(invalid("EP1 MP4 ambiguous source timestamp"));
            }
            streams
                .try_reserve(1)
                .map_err(|_| invalid("EP1 MP4 stream table allocation failed"))?;
            streams.push(Stream {
                index: at,
                id: track.id,
                es_id: d.es_id,
                parent: d.depends_on,
                timescale: track.timescale,
                configuration: track.configuration.clone(),
                samples,
            });
            let mut next = None;
            for (index, candidate) in tracks.iter().enumerate() {
                if candidate.handler != *b"soun" || candidate.codec != *b"mp4a" {
                    continue;
                }
                let child = aac_stream_descriptor(&candidate.configuration)?;
                if child.es_id == d.es_id && index != at {
                    return Err(invalid("EP1 MP4 duplicate ES ID"));
                }
                if child.depends_on == Some(d.es_id) {
                    if next.replace(index).is_some() {
                        return Err(invalid("EP1 MP4 forked ES dependency"));
                    }
                }
            }
            let Some(next) = next else {
                break;
            };
            at = next;
        }
        Ok(Self { root, streams })
    }
    pub fn root_track(&self) -> usize {
        self.root
    }
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }
    /// Read the complete dependency group at the selected root sample's exact
    /// rational source time. A missing or differently timed AU is never padded
    /// or substituted. No decoder state is touched by partial I/O failure.
    pub fn read_frame<R: Read + Seek>(
        &self,
        reader: &mut Mp4Reader<R>,
        root_sample: usize,
    ) -> Result<Vec<AccessUnit>> {
        let root = reader
            .tracks()
            .get(self.root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        let stamp = root
            .samples
            .get(root_sample)
            .ok_or_else(|| invalid("EP1 MP4 root sample index out of range"))?;
        let time = key(stamp.pts, root.timescale);
        let root_scale = root.timescale;
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.streams.len())
            .map_err(|_| invalid("EP1 MP4 AU table allocation failed"))?;
        for stream in &self.streams {
            let track = reader
                .tracks()
                .get(stream.index)
                .ok_or_else(|| invalid("EP1 MP4 stream track index out of range"))?;
            if track.id != stream.id
                || track.timescale != stream.timescale
                || track.configuration != stream.configuration
                || track.samples.len() != stream.samples.len()
            {
                return Err(invalid(
                    "EP1 MP4 group belongs to different track configuration",
                ));
            }
            let found = stream
                .samples
                .binary_search_by_key(&time, |entry| entry.0)
                .map_err(|_| invalid("EP1 MP4 missing simultaneous ESC access unit"))?;
            let index = stream.samples[found].1;
            let sample = track
                .samples
                .get(index)
                .ok_or_else(|| invalid("EP1 MP4 sample index out of range"))?;
            if key(sample.pts, track.timescale) != time
                || u128::from(sample.duration) * u128::from(root_scale)
                    != u128::from(stamp.duration) * u128::from(track.timescale)
            {
                return Err(invalid("EP1 MP4 ESC source clocks disagree"));
            }
            let bit_len = reader
                .packet_bit_len(stream.index, index)
                .map_err(|e| invalid(&e.to_string()))?;
            let mut data = Vec::new();
            reader
                .read_packet(stream.index, index, &mut data)
                .map_err(|e| invalid(&e.to_string()))?;
            output.push(AccessUnit {
                stream_id: stream.es_id,
                depends_on: stream.parent,
                data,
                bit_len,
                pts: sample.pts,
                duration: u64::from(sample.duration),
                timescale: stream.timescale,
            });
        }
        Ok(output)
    }
}
