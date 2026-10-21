//! Owned E.2.8 presentation assembly. Speaker identities survive coded ordering.
use super::*;
use crate::audio::DecodedAudio;
use std::collections::{BTreeSet, VecDeque};

/// First fifteen entries follow WAVE mask bit order; the remaining SMPTE
/// positions need an extended device map and are never aliased to another role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Speaker {
    Left,
    Right,
    Centre,
    Lfe,
    LeftRear,
    RightRear,
    LeftCentre,
    RightCentre,
    BackCentre,
    LeftSurround,
    RightSurround,
    TopCentre,
    HeightLeft,
    HeightCentre,
    HeightRight,
    TopSurroundLeft,
    TopSurroundRight,
    SurroundDirectLeft,
    SurroundDirectRight,
    WideLeft,
    WideRight,
    Lfe2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerLayout(pub Vec<Speaker>);
impl SpeakerLayout {
    pub fn wave_mask(&self) -> Option<u32> {
        let mut mask = 0;
        for &s in &self.0 {
            let bit = s as u32;
            if bit >= 15 {
                return None;
            }
            mask |= 1 << bit;
        }
        Some(mask)
    }
}
#[derive(Clone)]
struct Descriptor {
    frame: Frame,
    id: u8,
    map: Option<u16>,
    last: bool,
}
fn descriptor(bytes: &[u8]) -> Result<Descriptor> {
    let (frame, id, map, last) = read_substream_header(&mut BitReader::new(bytes), true)?;
    Ok(Descriptor {
        frame,
        id,
        map,
        last,
    })
}
fn envelope(bytes: &[u8]) -> Result<(u8, u8, usize)> {
    if bytes.len() < 6 || bytes[..2] != [0x0b, 0x77] || !(11..=16).contains(&(bytes[5] >> 3)) {
        return Err(invalid("invalid E-AC-3 presentation envelope"));
    }
    let kind = bytes[2] >> 6;
    let id = (bytes[2] >> 3) & 7;
    if kind == 3 {
        return Err(unsupported("reserved E-AC-3 presentation stream type"));
    }
    let size = (usize::from(bytes[2] & 7) * 256 + usize::from(bytes[3]) + 1) * 2;
    if size < 6 || size > bytes.len() {
        return Err(invalid("truncated E-AC-3 presentation substream"));
    }
    Ok((kind, id, size))
}
fn locations(d: &Descriptor) -> Vec<Speaker> {
    use Speaker::*;
    if let Some(map) = d.map {
        let entries: &[&[Speaker]] = &[
            &[Left],
            &[Centre],
            &[Right],
            &[LeftSurround],
            &[RightSurround],
            &[LeftCentre, RightCentre],
            &[LeftRear, RightRear],
            &[BackCentre],
            &[TopCentre],
            &[SurroundDirectLeft, SurroundDirectRight],
            &[WideLeft, WideRight],
            &[HeightLeft, HeightRight],
            &[HeightCentre],
            &[TopSurroundLeft, TopSurroundRight],
            &[Lfe2],
            &[Lfe],
        ];
        return entries
            .iter()
            .enumerate()
            .filter(|(i, _)| map & (1 << (15 - i)) != 0)
            .flat_map(|(_, s)| s.iter().copied())
            .collect();
    }
    let mut out = match d.frame.layout {
        0 | 2 => vec![Left, Right],
        1 => vec![Centre],
        3 => vec![Left, Centre, Right],
        4 => vec![Left, Right, BackCentre],
        5 => vec![Left, Centre, Right, BackCentre],
        6 => vec![Left, Right, LeftSurround, RightSurround],
        _ => vec![Left, Centre, Right, LeftSurround, RightSurround],
    };
    if d.frame.low_frequency {
        out.push(Lfe);
    }
    out
}
fn layout(descriptors: &[Descriptor], width: u16) -> Result<SpeakerLayout> {
    let speakers: Vec<_> = descriptors
        .iter()
        .flat_map(locations)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if speakers.len() != usize::from(width) || speakers.len() > 16 {
        return Err(invalid(
            "E-AC-3 presentation speaker count disagrees with configured output",
        ));
    }
    Ok(SpeakerLayout(speakers))
}
/// Inspect a complete first programme for container/device speaker assignment.
pub fn packet_speakers(data: &[u8], width: u16) -> Result<SpeakerLayout> {
    try_packet_speakers(data, width)?
        .ok_or_else(|| invalid("incomplete E-AC-3 dependent presentation"))
}
pub(crate) fn try_packet_speakers(data: &[u8], width: u16) -> Result<Option<SpeakerLayout>> {
    if !(7..=16).contains(&width) {
        return Err(invalid("expanded E-AC-3 output requires 7..16 channels"));
    }
    first_programme(data, false)?
        .map(|(_, frames)| layout(&frames, width))
        .transpose()
}
/// A complete native programme prefix for an elementary-file packet index.
/// None means that the independent stream has no associated dependent frame.
pub(crate) fn raw_programme(data: &[u8]) -> Result<Option<(usize, SpeakerLayout)>> {
    first_programme(data, true)?
        .map(|(size, frames)| {
            let count = frames
                .iter()
                .flat_map(locations)
                .collect::<BTreeSet<_>>()
                .len();
            Ok((size, layout(&frames, count as u16)?))
        })
        .transpose()
}
fn first_programme(data: &[u8], base_only: bool) -> Result<Option<(usize, Vec<Descriptor>)>> {
    let mut offset = 0;
    let mut frames = vec![];
    while offset < data.len() {
        let (kind, id, size) = envelope(&data[offset..])?;
        let raw = &data[offset..offset + size];
        offset += size;
        if frames.is_empty() {
            if kind == 1 || id != 0 {
                return Err(invalid(
                    "E-AC-3 presentation must start with independent programme 0",
                ));
            }
        } else if kind != 1 {
            if base_only && frames.len() == 1 {
                return Ok(None);
            }
            return Err(invalid("incomplete E-AC-3 dependent presentation"));
        }
        let d = descriptor(raw)?;
        if kind == 1 && usize::from(id) != frames.len() - 1 {
            return Err(invalid("non-sequential E-AC-3 dependent ID"));
        }
        if let Some(base) = frames.first() {
            let base: &Descriptor = base;
            if (d.frame.sample_rate, d.frame.blocks) != (base.frame.sample_rate, base.frame.blocks)
            {
                return Err(invalid(
                    "E-AC-3 dependent timing disagrees with independent stream",
                ));
            }
        }
        let done = kind == 1 && d.last;
        frames.push(d);
        if done {
            return Ok(Some((offset, frames)));
        }
    }
    if base_only && frames.len() > 1 {
        return Err(invalid("incomplete E-AC-3 dependent presentation"));
    }
    Ok(None)
}
#[derive(Clone, Copy)]
struct Origin {
    serial: u64,
    pts: i64,
    duration: u64,
}
struct Pending {
    origin: Origin,
    frames: Vec<(Descriptor, Vec<u8>)>,
}
struct Stream {
    decoder: Eac3Decoder,
    speakers: Vec<Speaker>,
}
struct Ready {
    origin: Origin,
    audio: DecodedAudio,
}

/// Assemble one programme, with replacement/extension channels and a single
/// final dynrng gain. Each constituent stream retains its own transform history.
pub struct PresentationDecoder {
    rate: u32,
    width: u16,
    lead: usize,
    initial_lead: usize,
    serial: u64,
    programme: Option<u8>,
    pending: Option<Pending>,
    ready: VecDeque<Ready>,
    streams: Vec<Option<Stream>>,
    speakers: Option<SpeakerLayout>,
}
impl PresentationDecoder {
    pub fn new(rate: u32, width: u16) -> Result<Self> {
        if !ALL_SAMPLE_RATES.contains(&rate) || !(7..=16).contains(&width) {
            return Err(invalid("invalid E-AC-3 presentation geometry"));
        }
        Ok(Self {
            rate,
            width,
            lead: ENCODER_DELAY,
            initial_lead: ENCODER_DELAY,
            serial: 0,
            programme: None,
            pending: None,
            ready: VecDeque::new(),
            streams: (0..9).map(|_| None).collect(),
            speakers: None,
        })
    }
    pub fn for_container(rate: u32, width: u16) -> Result<Self> {
        let mut s = Self::new(rate, width)?;
        s.set_priming(0);
        Ok(s)
    }
    pub(super) fn set_priming(&mut self, n: usize) {
        self.lead = n;
        self.initial_lead = n;
    }
    pub fn speakers(&self) -> Option<&SpeakerLayout> {
        self.speakers.as_ref()
    }
    fn assemble(&mut self, pending: Pending) -> Result<()> {
        let headers: Vec<_> = pending.frames.iter().map(|(h, _)| h.clone()).collect();
        let order = layout(&headers, self.width)?;
        if self.speakers.as_ref().is_some_and(|s| s != &order) {
            return Err(unsupported("changing E-AC-3 presentation speaker layout"));
        }
        let samples = headers[0].frame.blocks * SAMPLES;
        let mut pcm = vec![];
        let mut gains = [[1.0; 2]; BLOCKS];
        for (i, (d, bytes)) in pending.frames.iter().enumerate() {
            let roles = locations(d);
            if d.frame.sample_rate != self.rate
                || !crate::codec::ac3_decoder::frame_crc_valid(bytes)
            {
                return Err(invalid(
                    "corrupt or wrong-rate E-AC-3 presentation substream",
                ));
            }
            if self.streams[i]
                .as_ref()
                .is_some_and(|s| s.speakers != roles)
            {
                return Err(unsupported("changing E-AC-3 substream speaker assignment"));
            }
            if self.streams[i].is_none() {
                let mut decoder = Eac3Decoder::new(&[], self.rate, d.frame.channels)?;
                decoder.assembly_mode = true;
                self.streams[i] = Some(Stream {
                    decoder,
                    speakers: roles.clone(),
                });
            }
            let child = &mut self.streams[i].as_mut().unwrap().decoder;
            let mut decoded = vec![];
            let mut bits = Bits::at(bytes, d.frame.audio_bit);
            if !child.syncframe(&d.frame, &mut bits, &mut decoded)?
                || decoded.len() != samples * roles.len()
            {
                return Err(invalid("invalid E-AC-3 presentation audio block"));
            }
            gains = child.frame_gains;
            pcm.push(decoded);
        }
        // Annex E.2.8.5 takes dynrng from the last dependent for the entire mix.
        if headers.last().unwrap().frame.layout == 0
            && gains[..headers[0].frame.blocks]
                .iter()
                .any(|g| g[0] != g[1])
        {
            return Err(unsupported("dual-programme E-AC-3 expanded-output DRC"));
        }
        let width = usize::from(self.width);
        let mut mixed = vec![0.0f32; samples * width];
        for (d, decoded) in headers.iter().zip(&pcm) {
            let roles = locations(d);
            for (coded, role) in roles.iter().enumerate() {
                let output = order.0.binary_search(role).unwrap();
                for n in 0..samples {
                    mixed[n * width + output] = decoded[n * roles.len() + coded];
                }
            }
        }
        for n in 0..samples {
            for ch in 0..width {
                mixed[n * width + ch] =
                    (mixed[n * width + ch] * gains[n / SAMPLES][0]).clamp(-1.0, 1.0);
            }
        }
        let skip = self.lead.min(samples);
        self.lead -= skip;
        let bytes: Vec<u8> = mixed[skip * width..]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let origin = pending.origin;
        if let Some(last) = self
            .ready
            .back_mut()
            .filter(|last| last.origin.serial == origin.serial)
        {
            last.audio.packet.data.extend(bytes);
        } else {
            self.ready.push_back(Ready {
                origin,
                audio: DecodedAudio {
                    packet: AudioPacket {
                        data: bytes,
                        pts: origin.pts.max(0) as u64,
                        timebase_num: 1,
                        timebase_den: self.rate,
                    },
                    source_pts: origin.pts,
                    source_duration: origin.duration,
                },
            });
        }
        self.speakers = Some(order);
        Ok(())
    }
    fn ingest(&mut self, data: &[u8], pts: i64, duration: u64) -> Result<()> {
        if data.is_empty() {
            return Err(invalid("empty E-AC-3 presentation packet"));
        }
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or_else(|| invalid("E-AC-3 packet counter overflow"))?;
        let origin = Origin {
            serial: self.serial,
            pts,
            duration,
        };
        let mut offset = 0;
        while offset < data.len() {
            let (kind, id, size) = envelope(&data[offset..])?;
            let raw = &data[offset..offset + size];
            offset += size;
            if kind != 1 {
                if self.pending.is_some() {
                    return Err(invalid("missing final E-AC-3 dependent substream"));
                }
                self.programme = Some(id);
                if id != 0 {
                    continue;
                }
                let d = descriptor(raw)?;
                self.pending = Some(Pending {
                    origin,
                    frames: vec![(d, raw.to_vec())],
                });
            } else if self.programme == Some(0) {
                let pending = self
                    .pending
                    .as_mut()
                    .ok_or_else(|| invalid("E-AC-3 dependent follows completed programme"))?;
                let d = descriptor(raw)?;
                if usize::from(d.id) != pending.frames.len() - 1 || pending.frames.len() >= 9 {
                    return Err(invalid("non-sequential E-AC-3 dependent ID"));
                }
                let base = &pending.frames[0].0.frame;
                if (d.frame.sample_rate, d.frame.blocks) != (base.sample_rate, base.blocks) {
                    return Err(invalid(
                        "E-AC-3 dependent timing disagrees with independent stream",
                    ));
                }
                let done = d.last;
                pending.frames.push((d, raw.to_vec()));
                if done {
                    let complete = self.pending.take().unwrap();
                    self.assemble(complete)?;
                }
            } else if self.programme.is_none() {
                return Err(invalid("E-AC-3 dependent has no independent programme"));
            }
        }
        Ok(())
    }
}
impl AudioDecode for PresentationDecoder {
    fn decode_packet(
        &mut self,
        data: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<DecodedAudio>> {
        self.ingest(data, pts, duration)?;
        Ok(self.ready.pop_front().map(|r| r.audio))
    }
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<AudioPacket>> {
        self.decode_packet(data, pts.min(i64::MAX as u64) as i64, duration)
            .map(|p| p.map(|p| p.packet))
    }
    fn finish_packet(&mut self) -> Result<Option<DecodedAudio>> {
        if let Some(r) = self.ready.pop_front() {
            return Ok(Some(r.audio));
        }
        if self.pending.is_some() {
            return Err(invalid("incomplete E-AC-3 dependent presentation at EOF"));
        }
        Ok(None)
    }
    fn reset(&mut self) {
        self.lead = self.initial_lead;
        self.serial = 0;
        self.programme = None;
        self.pending = None;
        self.ready.clear();
        self.streams = (0..9).map(|_| None).collect();
        self.speakers = None;
    }
}
