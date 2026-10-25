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
    /// Canonical native output map, including replacement-only programmes. A fixed superset
    /// preserves role identity when frame-level channel maps change. Missing
    /// roles are silent; active roles outside this map must be refused.
    pub fn validate_output(&self) -> Result<()> {
        if !(1..=16).contains(&self.0.len()) || !self.0.windows(2).all(|w|w[0]<w[1]) {
            return Err(invalid("E-AC-3 output layout requires 1..16 unique speakers in canonical order"));
        }
        Ok(())
    }

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
/// Parse the selected independent programme's stable ISO BMFF speaker map.
/// ETSI TS 102 366 F.6: chan_loc uses LSB-first location indices, unlike chanmap.
/// Empty configuration retains the packet-probed legacy path.
pub fn configuration_speakers(data: &[u8], selected: u8) -> Result<Option<SpeakerLayout>> {
    use Speaker::*;
    if data.is_empty() { return Ok(None); }
    let mut bits = BitReader::new(data);
    bits.read(13)?;
    let last = bits.read(3)? as u8;
    let mut found = None;
    for id in 0..=last {
        bits.read(2)?;
        let bsid = bits.read(5)?;
        if !(11..=16).contains(&bsid) { return Err(invalid("invalid dec3 bitstream ID")); }
        bits.read(5)?;
        let mode = bits.read(3)?;
        let lfe = bits.read(1)? != 0;
        bits.read(3)?;
        let dependent = bits.read(4)?;
        let extra = if dependent != 0 { bits.read(9)? } else { bits.read(1)?; 0 };
        if id != selected { continue; }
        let mut roles = match mode {
            0 | 2 => vec![Left, Right],
            1 => vec![Centre],
            3 => vec![Left, Centre, Right],
            4 => vec![Left, Right, BackCentre],
            5 => vec![Left, Centre, Right, BackCentre],
            6 => vec![Left, Right, LeftSurround, RightSurround],
            _ => vec![Left, Centre, Right, LeftSurround, RightSurround],
        };
        if lfe { roles.push(Lfe); }
        let extensions: &[&[Speaker]] = &[
            &[LeftCentre,RightCentre], &[LeftRear,RightRear], &[BackCentre],
            &[TopCentre], &[SurroundDirectLeft,SurroundDirectRight],
            &[WideLeft,WideRight], &[HeightLeft,HeightRight], &[HeightCentre], &[Lfe2],
        ];
        for (bit, speakers) in extensions.iter().enumerate() {
            if extra & (1 << bit) != 0 { roles.extend_from_slice(speakers); }
        }
        roles.sort(); roles.dedup();
        let layout = SpeakerLayout(roles);
        if dependent != 0 || layout.0.len() > 6 { layout.validate_output()?; found = Some(layout); }
    }
    Ok(found)
}
#[derive(Clone)]
struct Descriptor {
    frame: Frame,
    id: u8,
    map: Option<u16>,
    last: bool,
}
fn descriptor(bytes: &[u8]) -> Result<Descriptor> {
    let canonical=crate::codec::ac3_decoder::dolby_frame(bytes)?;
    let (frame, id, map, last) = read_substream_header(&mut BitReader::new(canonical.as_ref()), true)?;
    Ok(Descriptor {
        frame,
        id,
        map,
        last,
    })
}
fn envelope(bytes: &[u8]) -> Result<(u8, u8, usize)> {
    let h=crate::codec::ac3_decoder::dolby_prefix(bytes).ok_or_else(||invalid("invalid E-AC-3 presentation envelope"))?;
    if h[..2] != [0x0b, 0x77] || !(11..=16).contains(&(h[5] >> 3)) {return Err(invalid("invalid E-AC-3 presentation envelope"));}
    let kind = h[2] >> 6;
    let id = (h[2] >> 3) & 7;
    if kind == 3 {
        return Err(unsupported("reserved E-AC-3 presentation stream type"));
    }
    let size = (usize::from(h[2] & 7) * 256 + usize::from(h[3]) + 1) * 2;
    if size < 6 || size > bytes.len() {
        return Err(invalid("truncated E-AC-3 presentation substream"));
    }
    Ok((kind, id, size))
}
fn locations(d: &Descriptor) -> Vec<Speaker> {
    mapped_locations(d.frame.layout, d.frame.low_frequency, d.map)
}
fn mapped_locations(mode: u8, lfe: bool, map: Option<u16>) -> Vec<Speaker> {
    use Speaker::*;
    if let Some(map) = map {
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
    let mut out = match mode {
        0 | 2 => vec![Left, Right],
        1 => vec![Centre],
        3 => vec![Left, Centre, Right],
        4 => vec![Left, Right, BackCentre],
        5 => vec![Left, Centre, Right, BackCentre],
        6 => vec![Left, Right, LeftSurround, RightSurround],
        _ => vec![Left, Centre, Right, LeftSurround, RightSurround],
    };
    if lfe {
        out.push(Lfe);
    }
    out
}
/// Speaker-only BSI probe; at most sixteen bytes, without entropy or CRC reads.
/// The ordinary decoder remains responsible for complete frame validation.
pub fn header_speakers(data: &[u8]) -> Result<SpeakerLayout> {
    let mut canonical = [0u8; 16];
    let n=data.len().min(canonical.len()); canonical[..n].copy_from_slice(&data[..n]);
    if canonical[..2] == [0x77,0x0b] {
        for word in canonical[..n].chunks_exact_mut(2) { word.swap(0,1); }
    }
    let mut b=BitReader::new(&canonical[..n]);
    if b.read(16)? != SYNCWORD { return Err(invalid("invalid E-AC-3 speaker header sync")); }
    let kind=b.read(2)?; b.read(3)?; b.read(11)?;
    if kind==3 { return Err(invalid("reserved E-AC-3 speaker header type")); }
    let rate=b.read(2)?; let second=b.read(2)?;
    if rate==3 && second==3 { return Err(invalid("reserved E-AC-3 speaker header rate")); }
    let mode=b.read(3)? as u8; let lfe=b.bit()?;
    if !(11..=16).contains(&b.read(5)?) { return Err(invalid("invalid E-AC-3 speaker header ID")); }
    b.read(5)?; if b.bit()? { b.read(8)?; }
    if mode==0 { b.read(5)?; if b.bit()? { b.read(8)?; } }
    let map=if kind==1 && b.bit()? { Some(b.read(16)? as u16) } else { None };
    if let Some(map)=map {
        let pairs=0x0400u16|0x0200|0x0040|0x0020|0x0010|0x0004;
        if map.count_ones()+(map & pairs).count_ones()!=NFCHANS[mode as usize] as u32+u32::from(lfe) {
            return Err(invalid("E-AC-3 speaker header map disagrees with coded channel count"));
        }
    }
    let mut roles=mapped_locations(mode,lfe,map); roles.sort(); roles.dedup();
    Ok(SpeakerLayout(roles))
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
/// Bounded-setup callers can accumulate complete substream frames until this
/// selected programme closes. Native-only programmes need the following
/// independent frame or EOF to distinguish a missing dependent from no extension.
pub(crate) fn probe_programme(data:&[u8],selected:u8,eof:bool)->Result<Option<(Frame,SpeakerLayout)>> {
    probe_native_programme(data,selected,eof).map(|p|p.map(|(frame,layout,_)|(frame,layout)))
}
/// Preserve whether a complete programme actually carries dependent components,
/// including replacement-only output that fits within the independent width.
pub(crate) fn probe_native_programme(data:&[u8],selected:u8,eof:bool)->Result<Option<(Frame,SpeakerLayout,bool)>> {
    if selected>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
    fn complete(frames:&[Descriptor])->Result<(Frame,SpeakerLayout,bool)> {
        let base=frames[0].frame;
        let count=frames.iter().flat_map(locations).collect::<BTreeSet<_>>().len();
        let speakers=layout(frames,count as u16)?;
        Ok((base,speakers,frames.len()>1))
    }
    let mut offset=0;let mut frames:Vec<Descriptor>=Vec::new();
    while offset<data.len() {
        let remaining=&data[offset..];
        if remaining.len()<6 {
            return if eof {Err(invalid("truncated E-AC-3 programme setup"))}else{Ok(None)};
        }
        let size=(usize::from(remaining[2]&7)*256+usize::from(remaining[3])+1)*2;
        if size>remaining.len(){return if eof{Err(invalid("truncated E-AC-3 programme setup"))}else{Ok(None)};}
        let(kind,id,size)=envelope(remaining)?;let raw=&remaining[..size];offset+=size;
        if frames.is_empty(){
            if kind==1 || id!=selected {continue;}
        }else if kind!=1 {
            return if frames.len()==1{complete(&frames).map(Some)}else{Err(invalid("missing final selected E-AC-3 dependent substream"))};
        }
        if !crate::codec::ac3_decoder::frame_crc_valid(raw){return Err(invalid("corrupt selected E-AC-3 programme setup"));}
        let d=descriptor(raw)?;
        if !frames.is_empty(){
            if usize::from(id)!=frames.len()-1 || frames.len()>=9{return Err(invalid("non-sequential E-AC-3 dependent ID"));}
            let base=&frames[0].frame;
            if (d.frame.sample_rate,d.frame.blocks)!=(base.sample_rate,base.blocks){return Err(invalid("E-AC-3 dependent timing disagrees with independent stream"));}
        }
        let done=kind==1&&d.last;frames.push(d);
        if done{return complete(&frames).map(Some);}
    }
    if eof && frames.len()==1{return complete(&frames).map(Some);}
    if eof && frames.len()>1{return Err(invalid("incomplete selected E-AC-3 programme setup"));}
    Ok(None)
}

/// Inspect a complete first programme for container/device speaker assignment.
pub fn packet_speakers(data: &[u8], width: u16) -> Result<SpeakerLayout> {
    try_packet_speakers(data, width)?
        .ok_or_else(|| invalid("incomplete E-AC-3 dependent presentation"))
}
/// Speaker assignment of a complete explicitly selected programme.
pub fn packet_speakers_for_programme(data:&[u8],width:u16,programme:u8)->Result<SpeakerLayout> {
    if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
    if !(1..=16).contains(&width){return Err(invalid("native E-AC-3 output requires 1..16 channels"));}
    first_programme(data,false,programme)?.map(|(_,frames)|layout(&frames,width)).transpose()?.ok_or_else(||invalid("incomplete selected E-AC-3 dependent presentation"))
}
pub(crate) fn try_packet_speakers(data: &[u8], width: u16) -> Result<Option<SpeakerLayout>> {
    if !(1..=16).contains(&width) {
        return Err(invalid("native E-AC-3 output requires 1..16 channels"));
    }
    first_programme(data, false, 0)?
        .map(|(_, frames)| layout(&frames, width))
        .transpose()
}
/// A complete native programme prefix for an elementary-file packet index.
/// None means that the independent stream has no associated dependent frame.
pub(crate) fn raw_programme(data: &[u8]) -> Result<Option<(usize, SpeakerLayout)>> {
    raw_programme_for_programme(data,0)
}
pub(crate) fn raw_programme_for_programme(data:&[u8],programme:u8)->Result<Option<(usize,SpeakerLayout)>> {
    if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
    first_programme(data, true, programme)?
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
fn first_programme(data: &[u8], base_only: bool, selected:u8) -> Result<Option<(usize, Vec<Descriptor>)>> {
    let mut offset = 0;
    let mut frames = vec![];
    while offset < data.len() {
        let (kind, id, size) = envelope(&data[offset..])?;
        let raw = &data[offset..offset + size];
        offset += size;
        if frames.is_empty() {
            if kind == 1 || id != selected {
                if selected!=0 {continue;}
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
    // Transform/transient history belongs to coded planes, not speaker labels.
    width: usize,
    native: transient::NativeBlocks<u64>,
}
struct ProgrammeWork {
    id: u64,
    origin: Origin,
    order: SpeakerLayout,
    roles: Vec<Vec<Speaker>>,
    dual: Vec<bool>,
    pcm: Vec<Vec<f32>>,
    samples: usize,
    gains: [[f32; 2]; BLOCKS],
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
    compression_mode: CompressionMode,
    selected_programme:u8,
    dialogue_normalization: bool,
    lead: usize,
    initial_lead: usize,
    serial: u64,
    programme: Option<u8>,
    pending: Option<Pending>,
    ready: VecDeque<Ready>,
    works: VecDeque<ProgrammeWork>,
    work_serial: u64,
    finished: bool,
    streams: Vec<Option<Stream>>,
    speakers: Option<SpeakerLayout>,
    output_layout: Option<SpeakerLayout>,
}
impl PresentationDecoder {
    pub fn new(rate: u32, width: u16) -> Result<Self> {
        if !ALL_SAMPLE_RATES.contains(&rate) || !(1..=16).contains(&width) {
            return Err(invalid("invalid E-AC-3 presentation geometry"));
        }
        Ok(Self {
            rate,
            width,
            compression_mode: CompressionMode::Line,
            selected_programme:0,
            dialogue_normalization: false,
            lead: ENCODER_DELAY,
            initial_lead: ENCODER_DELAY,
            serial: 0,
            programme: None,
            pending: None,
            ready: VecDeque::new(),
            works: VecDeque::new(),
            work_serial: 0,
            finished: false,
            streams: (0..9).map(|_| None).collect(),
            speakers: None,
            output_layout: None,
        })
    }
    pub fn for_container(rate: u32, width: u16) -> Result<Self> {
        let mut s = Self::new(rate, width)?;
        s.set_priming(0);
        Ok(s)
    }
    /// Select independent programme ID 0..7 (zero by default).
    /// Changing the choice clears pending PCM and history; reset retains it.
    pub fn with_programme(mut self, programme:u8)->Result<Self> {
        self.select_programme(programme)?;Ok(self)
    }
    pub(super) fn select_programme(&mut self,programme:u8)->Result<()> {
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        if self.selected_programme!=programme {self.reset();self.selected_programme=programme;}
        Ok(())
    }

    pub(super) fn set_compression(&mut self, mode: CompressionMode) {
        self.compression_mode=mode;
    }
    /// Enable decoder-side BSI dialogue normalization; reset preserves the choice.
    pub fn with_dialogue_normalization(mut self, enabled: bool) -> Self {
        self.set_dialogue_normalization(enabled);
        self
    }
    pub(super) fn set_dialogue_normalization(&mut self, enabled: bool) {
        self.dialogue_normalization=enabled;
    }
    pub(super) fn set_priming(&mut self, n: usize) {
        self.lead = n;
        self.initial_lead = n;
    }
    /// Configure a stable output superset before decoding. All active speaker
    /// roles must belong to it; its length must match the configured PCM width.
    pub fn with_output_layout(mut self, layout: SpeakerLayout) -> Result<Self> {
        self.set_output_layout(layout)?;
        Ok(self)
    }
    pub(super) fn set_output_layout(&mut self, layout: SpeakerLayout) -> Result<()> {
        layout.validate_output()?;
        if layout.0.len()!=usize::from(self.width) || self.serial!=0 {
            return Err(invalid("E-AC-3 output layout must match configured width and precede decoding"));
        }
        self.speakers=Some(layout.clone());
        self.output_layout=Some(layout);
        Ok(())
    }
    /// Speaker order of emitted PCM; an explicit superset is known before input.
    pub fn speakers(&self) -> Option<&SpeakerLayout> {
        self.speakers.as_ref()
    }
    fn assemble(&mut self, pending: Pending) -> Result<()> {
        let headers: Vec<_> = pending.frames.iter().map(|(h, _)| h.clone()).collect();
        let order = if let Some(target)=&self.output_layout {
            let count=headers.iter().flat_map(locations).collect::<BTreeSet<_>>().len();
            let active=layout(&headers,count as u16)?;
            if active.0.iter().any(|s|target.0.binary_search(s).is_err()) {
                return Err(invalid("E-AC-3 programme speaker is outside negotiated output layout"));
            }
            target.clone()
        } else {layout(&headers, self.width)?};
        if self.speakers.as_ref().is_some_and(|s| s != &order) {
            return Err(unsupported("changing E-AC-3 presentation speaker layout"));
        }
        let blocks = headers[0].frame.blocks;
        let samples = blocks * SAMPLES;
        let mut reconstructed = Vec::new();
        let mut all_roles = Vec::new();
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
            if self.streams[i].is_none() {
                let mut decoder = Eac3Decoder::new(&[], self.rate, d.frame.channels)?;
                decoder.assembly_mode = true;
                self.streams[i] = Some(Stream {
                    decoder,
                    width: roles.len(),
                    native: Default::default(),
                });
            }
            let stream = self.streams[i].as_mut().unwrap();
            stream.width = roles.len();
            let child = &mut stream.decoder;
            child.channels = roles.len();
            child.compression_mode = self.compression_mode;
            child.dialogue_normalization = self.dialogue_normalization;
            let mut decoded = Vec::new();
            let canonical=crate::codec::ac3_decoder::dolby_frame(bytes)?;
            let mut bits = Bits::at(canonical.as_ref(), d.frame.audio_bit);
            child.native_capture = Some(Vec::new());
            let sound = child.syncframe(&d.frame, &mut bits, &mut decoded);
            // Restore the capture switch even when syntax decoding errors.
            let native = child.native_capture.take().unwrap();
            if !sound? || native.len() != blocks || decoded.len() != samples * roles.len() {
                return Err(invalid("invalid E-AC-3 presentation audio block"));
            }
            gains = child.frame_gains;
            reconstructed.push(native);
            all_roles.push(roles);
        }
        // The last dependent supplies normal DRC for the complete programme.
        if headers.last().unwrap().frame.layout != 0 {
            // A single-programme last dependent supplies one common DRC word.
            for gain in &mut gains[..blocks] {
                gain[1] = gain[0];
            }
        }
        self.work_serial = self
            .work_serial
            .checked_add(1)
            .ok_or_else(|| invalid("E-AC-3 programme counter overflow"))?;
        let id = self.work_serial;
        let participating = reconstructed.len();
        self.works.push_back(ProgrammeWork {
            id,
            origin: pending.origin,
            order: order.clone(),
            pcm: all_roles
                .iter()
                .map(|r| Vec::with_capacity(samples * r.len()))
                .collect(),
            // Snapshot this programme's maps: delayed correction of an earlier
            // programme must not adopt a later frame's speaker assignment.
            roles: all_roles,
            dual: headers.iter().map(|h| h.frame.layout == 0).collect(),
            samples,
            gains,
        });
        for (i, blocks) in reconstructed.into_iter().enumerate() {
            let mut output = Vec::new();
            let stream = self.streams[i].as_mut().unwrap();
            for b in blocks {
                output.extend(
                    stream
                        .native
                        .push(&b.pcm, b.windows, b.mixing, b.gains, b.clip, id)?,
                );
            }
            self.accept(i, output)?;
        }
        // A temporarily absent replacement stream must not strand an earlier
        // programme. Advance its PCM clock with silent, non-presented blocks.
        // Its transform state retains the prior decoder's absence behaviour.
        for i in participating..self.streams.len() {
            let Some(stream) = self.streams[i].as_mut() else {
                continue;
            };
            let silent = transient::NativeBlock::silence(stream.width);
            let mut output = Vec::new();
            for _ in 0..blocks {
                output.extend(stream.native.push(
                    &silent.pcm,
                    silent.windows,
                    silent.mixing,
                    silent.gains,
                    silent.clip,
                    0,
                )?);
            }
            self.accept(i, output)?;
        }
        self.speakers = Some(order);
        self.complete_programmes()
    }
    fn accept(&mut self, component: usize, output: Vec<(u64, Vec<f32>)>) -> Result<()> {
        for (id, pcm) in output {
            if id == 0 {
                continue;
            }
            let work = self
                .works
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or_else(|| invalid("E-AC-3 component lost its source programme"))?;
            let roles = work
                .roles
                .get(component)
                .ok_or_else(|| invalid("unexpected E-AC-3 component output"))?;
            let target = work.samples * roles.len();
            let plane = &mut work.pcm[component];
            if pcm.len() != SAMPLES * roles.len() || plane.len() + pcm.len() > target {
                return Err(invalid("E-AC-3 component output exceeds programme window"));
            }
            plane.extend(pcm);
        }
        Ok(())
    }
    fn complete_programmes(&mut self) -> Result<()> {
        while self.works.front().is_some_and(|w| {
            w.pcm
                .iter()
                .zip(&w.roles)
                .all(|(pcm, roles)| pcm.len() == w.samples * roles.len())
        }) {
            let work = self.works.pop_front().unwrap();
            let width = usize::from(self.width);
            let mut mixed = vec![0.0; work.samples * width];
            let mut programme = vec![0; width];
            for ((roles, pcm), &dual) in work.roles.iter().zip(&work.pcm).zip(&work.dual) {
                for (coded, role) in roles.iter().enumerate() {
                    let output = work.order.0.binary_search(role).unwrap();
                    // DRC ownership follows the component that wins replacement,
                    // not the physical left/right speaker number. Apply it once,
                    // after correction and channel assembly, before final clip.
                    programme[output] = usize::from(dual && coded == 1);
                    for n in 0..work.samples {
                        mixed[n * width + output] = pcm[n * roles.len() + coded];
                    }
                }
            }
            for n in 0..work.samples {
                for ch in 0..width {
                    mixed[n * width + ch] =
                        (mixed[n * width + ch] * work.gains[n / SAMPLES][programme[ch]]).clamp(-1.0, 1.0);
                }
            }
            let skip = self.lead.min(work.samples);
            self.lead -= skip;
            let bytes = mixed[skip * width..]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>();
            let origin = work.origin;
            if let Some(last) = self
                .ready
                .back_mut()
                .filter(|r| r.origin.serial == origin.serial)
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
        }
        Ok(())
    }
    fn take_ready(&mut self) -> Option<DecodedAudio> {
        let serial = self.ready.front()?.origin.serial;
        // A single input can contain several programmes. Do not expose a
        // partial source window while its last programme still has lookahead.
        if self.works.iter().any(|w| w.origin.serial == serial)
            || self
                .pending
                .as_ref()
                .is_some_and(|p| p.origin.serial == serial)
        {
            return None;
        }
        self.ready.pop_front().map(|r| r.audio)
    }
    fn ingest(&mut self, data: &[u8], pts: i64, duration: u64) -> Result<()> {
        if self.finished {
            return Err(invalid("E-AC-3 presentation packet after EOF"));
        }
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
                if id != self.selected_programme {
                    continue;
                }
                let d = descriptor(raw)?;
                self.pending = Some(Pending {
                    origin,
                    frames: vec![(d, raw.to_vec())],
                });
            } else if self.programme == Some(self.selected_programme) {
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
    fn set_dolby_dialogue_normalization(&mut self, enabled: bool) {
        self.set_dialogue_normalization(enabled);
    }
    fn set_dolby_compression(&mut self, mode: CompressionMode) {
        self.set_compression(mode);
    }

    fn decode_packet(
        &mut self,
        data: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<DecodedAudio>> {
        self.ingest(data, pts, duration)?;
        Ok(self.take_ready())
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
        if let Some(audio) = self.take_ready() {
            return Ok(Some(audio));
        }
        if self.pending.is_some() {
            return Err(invalid("incomplete E-AC-3 dependent presentation at EOF"));
        }
        if !self.finished {
            for i in 0..self.streams.len() {
                if let Some(stream) = self.streams[i].as_mut() {
                    let output = stream.native.finish()?;
                    self.accept(i, output)?;
                }
            }
            self.complete_programmes()?;
            if !self.works.is_empty() {
                return Err(invalid("incomplete E-AC-3 corrected programme at EOF"));
            }
            self.finished = true;
        }
        Ok(self.take_ready())
    }
    fn reset(&mut self) {
        self.lead = self.initial_lead;
        self.serial = 0;
        self.programme = None;
        self.pending = None;
        self.ready.clear();
        self.works.clear();
        self.work_serial = 0;
        self.finished = false;
        self.streams = (0..9).map(|_| None).collect();
        self.speakers = self.output_layout.clone();
    }
}
