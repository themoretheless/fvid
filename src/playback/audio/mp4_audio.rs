//! Audio packet extraction from MP4 containers.
//!
//! The audio reader extracts packets from an AAC or uncompressed PCM track and
//! provides them to a decoder. Packet timestamps remain in the track media
//! timeline.

use crate::container::mp4::{Limits, Mp4Reader, Track};
use crate::{Result, invalid, unsupported};
use std::io::{Read, Seek};

/// Audio packet with presentation timestamp.
pub struct AudioPacket {
    /// Encoded audio data (AAC frame, etc.).
    pub data: Vec<u8>,
    /// Presentation timestamp in track timescale units.
    pub pts: i64,
    /// Duration in track timescale units.
    pub duration: i64,
    /// Sample index within the track.
    pub sample_index: usize,
}

#[derive(Clone, Default)]
struct DescriptionState { rate: Option<u32>, channels: Option<u16>, mask: Option<u32>, in_band_ps: bool, native_layout: Option<crate::codec::eac3_decoder::presentation::SpeakerLayout> }
#[path = "mp4_edit_clock.rs"]
mod edit_clock;
use edit_clock::{Time as EditTime, Timeline as VariableTimeline};
#[path="mp4_clock_recovery.rs"] mod clock_recovery;
use clock_recovery::PlaybackClock;
enum EditAction { Encoded, Step(crate::audio::AudioStep), End }
/// MP4 audio track reader. Extracts encoded packets for decoding.
pub struct Mp4AudioReader<R> {
    navigation_error: Option<crate::Error>,
    clock: Option<PlaybackClock>,
    clock_mode: std::cell::Cell<bool>,
    clock_floor: std::cell::Cell<EditTime>,
    clock_anchor: std::cell::Cell<Option<(i64,EditTime)>>,
    variable_edits: Option<VariableTimeline>,
    opening_sample_index: usize,
    changing: bool,
    active_description: usize,
    configuration_pending: bool,
    description_states: Vec<Option<DescriptionState>>,
    demuxer: Mp4Reader<R>,
    track_index: usize,
    in_band_ps: bool,
    dolby_mask: Option<u32>,
    dolby_channels: Option<u16>,
    dolby_rate:Option<u32>,
    eac3_programme:u8,
    eac3_stereo_output: bool,
    eac3_output_layout: Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>,
    eac3_packet_layout: Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>,
    ac3_stereo_output: bool,
    dolby_track_channels: Vec<Option<u16>>,
    sample_index: usize,
    presentation_floor: i64,
    packet: Vec<u8>,
    timeline: Option<crate::container::audio_timeline::AudioTimeline>,
    edit_cursor: std::cell::Cell<u64>,
    edit_segment: std::cell::Cell<Option<usize>>,
}

/// Audio fourccs decoded by the player, including the owned PCM endian mapping.
const CODECS: [[u8; 4]; 16] = [
    *b"mp4a",
    *b"raw ",
    *b"in24",
    *b"in32",
    *b"sowt",
    *b"twos",
    *b"fl32",
    *b"fl64",
    *b"ima4",
    *b"ms\x00\x02",
    *b"ms\x00\x11",
    *b"alac",
    *b"MAC3",
    *b"MAC6",
    *b"ac-3",
    *b"ec-3",
];

/// The decoder dispatch's name for a tag that cannot spell it itself. Not just the
/// ADPCM trio needs this: `ms\0\xNN` is not text at all, and both the player and
/// the coverage gate know these codings by what they code, not by four bytes. MACE's
/// two are spelled `MAC3` and `MAC6` in their entries: text, but not the names the
/// dispatch matches on.
fn codec_name(codec: &[u8; 4]) -> Option<&'static str> {
    Some(match codec {
        b"ima4" => "adpcm_ima_qt",
        b"ms\x00\x02" => "adpcm_ms",
        b"ms\x00\x11" => "adpcm_ima_wav",
        b"MAC3" => "mace3",
        b"MAC6" => "mace6",
        _ => return None,
    })
}

impl<R: Read + Seek> Mp4AudioReader<R> {
    /// Prefer enabled movie audio; explicit track lists retain container order.
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        let demuxer = Mp4Reader::open_indexed(reader, limits)?;
        let candidates = Self::supported(&demuxer);
        let usable: Vec<_> = candidates.iter().copied()
            .filter(|&index| Self::automatic_pcm_usable(&demuxer.tracks()[index])).collect();
        let admitted = if usable.is_empty() { &candidates } else { &usable };
        let rank = |index| match demuxer.track_header_flags(index).unwrap_or(0) & 3 {
            3 => 2, 1 => 1, _ => 0,
        };
        if let Some(best) = admitted.iter().map(|&index| rank(index)).max()
            && let Some(index) = admitted.iter().copied().find(|&index| rank(index) == best)
        {
            return Self::from_demuxer(demuxer,index);
        }
        match Self::first_audio(&demuxer) {
            Some(codec) => Err(unsupported(&format!("MP4 audio track coded `{}` has no decoder here",String::from_utf8_lossy(&codec)))),
            None => Err(invalid("MP4 has no such audio track")),
        }
    }

    /// Open the `nth` audio track a decoder exists for, counting in container
    /// order. Which one the player wants is the player's choice; the container
    /// only reports the ones a decoder exists for.
    pub fn open_at(reader: R, limits: Limits, nth: usize) -> Result<Self> {
        let demuxer = Mp4Reader::open_indexed(reader, limits)?;
        let index = match Self::supported(&demuxer).get(nth).copied() {
            Some(index) => index,
            // An audio track the container listed says what it is coded as, so a
            // refusal that names one is the player having no arm for that coding -
            // not a file it failed to read.
            None => match Self::first_audio(&demuxer) {
                Some(codec) => {
                    return Err(unsupported(&format!(
                        "MP4 audio track coded `{}` has no decoder here",
                        String::from_utf8_lossy(&codec)
                    )));
                }
                None => return Err(invalid("MP4 has no such audio track")),
            },
        };
        Self::from_demuxer(demuxer, index)
    }

    /// Open encoded E-AC-3 programme 0..7 inside a zero-based physical audio track.
    pub fn open_programme(reader:R,limits:Limits,nth:usize,programme:u8)->Result<Self>{
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        let demuxer=Mp4Reader::open_indexed(reader,limits)?;
        let index=Self::supported(&demuxer).get(nth).copied().ok_or_else(||invalid("MP4 has no such audio track"))?;
        Self::from_programme(demuxer,index,Some(programme))
    }

    /// Fourcc of the first audio track the container found, whether or not a
    /// decoder exists for it: one this reader indexed, or one it read to the end
    /// of the sample entry and set aside for want of an arm.
    fn first_audio(demuxer: &Mp4Reader<R>) -> Option<[u8; 4]> {
        demuxer
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .map(|t| t.codec)
            .or_else(|| {
                demuxer
                    .refused()
                    .iter()
                    .find(|t| t.handler == *b"soun")
                    .map(|t| t.codec)
            })
    }

    /// Container indices of the audio tracks a decoder exists for.
    fn automatic_pcm_usable(track: &crate::container::mp4::Track) -> bool {
        if !matches!(&track.codec, b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64") {
            return true;
        }
        crate::codec::make_audio_decoder(std::str::from_utf8(&track.codec).unwrap_or(""),
            &track.configuration, track.sample_rate, track.channels, track.bit_depth).is_ok()
    }

    fn supported(demuxer: &Mp4Reader<R>) -> Vec<usize> {
        demuxer
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"soun" && CODECS.contains(&t.codec))
            .map(|(index, _)| index)
            .collect()
    }

    fn description_speakers(demuxer:&mut Mp4Reader<R>,track:usize,description:usize,programme:u8)
        ->Result<Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>> {
        let (mut sample,mut offset)=(0,0);
        crate::codec::eac3_decoder::presentation::scan_stream_speakers(programme,|wanted,mut out| {
            let mut done=0;
            while done<wanted {
                let Some(packet)=demuxer.tracks()[track].samples.get(sample) else {break;};
                let size=packet.size as usize;
                if packet.description_index as usize!=description+1 || offset>=size {sample+=1;offset=0;continue;}
                let n=(wanted-done).min(size-offset);
                if let Some(bytes)=out.as_deref_mut() {
                    if demuxer.read_packet_range(track,sample,offset,&mut bytes[done..done+n])?!=n {
                        return Err(invalid("incomplete E-AC-3 MP4 header range"));
                    }
                }
                done+=n;offset+=n;
            }
            Ok(done)
        })
    }

    /// Probe Dolby packet geometry before exposing track descriptions. A sample
    /// entry can carry a placeholder channel count instead of decoded roles.
    fn dolby_geometry(demuxer: &mut Mp4Reader<R>, index: usize) -> Result<Option<(u16,Option<u32>,Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>)>> {
        let track=&demuxer.tracks()[index];
        let kind=track.codec;
        if track.samples.is_empty() || !matches!(&kind,b"ac-3"|b"ec-3") {return Ok(None);}
        let mut payload=Vec::new();demuxer.read_packet(index,0,&mut payload)?;
        if kind==*b"ac-3" && let Some(frame)=crate::codec::ac3_decoder::checked_syncframe(&payload) {
            return Ok(Some((frame.channels,Some(crate::codec::ac3_decoder::speaker_mask(&payload,frame.channels)?),None)));
        }
        if kind==*b"ec-3" && let Some(frame)=crate::codec::eac3_decoder::checked_frame(&payload) {
            let layout=crate::codec::eac3_decoder::presentation::raw_programme(&payload)?;
            let native_channels=layout.as_ref().map_or(frame.channels,|(_,layout)|layout.0.len() as u16);
            let mask=if native_channels>6 {layout.as_ref().and_then(|(_,l)|l.wave_mask())}else{Some(crate::codec::eac3_decoder::speaker_mask(&payload,native_channels)?)};
            return Ok(Some((native_channels,mask,layout.map(|(_,l)|l))));
        }
        Ok(None)
    }

    fn programme_geometry(demuxer:&mut Mp4Reader<R>,index:usize,programme:u8)->Result<(u32,u16,Option<u32>,Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>)> {
        const FRAMES:usize=2*8*9;const BYTES:usize=FRAMES*4096;
        let count=demuxer.tracks()[index].samples.len();
        let mut prefix=Vec::new();let mut packet=Vec::new();
        for sample in 0..count.min(FRAMES) {
            demuxer.read_packet(index,sample,&mut packet)?;
            prefix.extend_from_slice(&packet[..packet.len().min(BYTES.saturating_sub(prefix.len()))]);
            if let Some((frame,layout,dependent))=crate::codec::eac3_decoder::presentation::probe_native_programme(&prefix,programme,sample+1==count)? {
                let mask=layout.wave_mask();
                let native=dependent.then(||layout.clone());
                return Ok((frame.sample_rate,layout.0.len()as u16,mask,native));
            }
            if prefix.len()==BYTES{break;}
        }
        Err(invalid("selected E-AC-3 programme is missing or incomplete in setup"))
    }

    /// Open a specific audio track by index.
    pub fn from_demuxer(demuxer:Mp4Reader<R>,index:usize)->Result<Self>{Self::from_programme(demuxer,index,None)}
    fn from_programme(mut demuxer: Mp4Reader<R>, index: usize,programme:Option<u8>) -> Result<Self> {
        let clock=PlaybackClock::build(&mut demuxer,index,programme.unwrap_or(0))?;
        if clock.is_some() || demuxer.tracks().get(index).is_some_and(|t|t.ensure_constant_description().is_err()) {
            return Self::from_changing(demuxer,index,programme,clock);
        }
        let selected_geometry=if let Some(programme)=programme {
            let track=demuxer.tracks().get(index).ok_or_else(||invalid("MP4 track index out of range"))?;
            if track.handler!=*b"soun" || track.codec!=*b"ec-3"{return Err(unsupported("programme selection requires an E-AC-3 track"));}
            Some(Self::programme_geometry(&mut demuxer,index,programme)?)
        }else{None};
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        track.ensure_constant_description()?;
        if track.handler != *b"soun" {
            return Err(invalid("selected MP4 track is not audio"));
        }
        let owned_adpcm = matches!(&track.codec, b"ima4" | b"ms\x00\x02" | b"ms\x00\x11");
        let timeline = if (track.codec == *b"mp4a" && (track.edits.len()>1 || track.edits.first().is_some_and(|e|e.media_time<0)))
            || ((owned_adpcm || matches!(&track.codec, b"ac-3" | b"ec-3" | b"MAC3" | b"MAC6")) && !track.edits.is_empty()) {
            Some(crate::container::audio_timeline::AudioTimeline::new(&track.edits,track.duration,track.timescale,demuxer.movie_timescale(),selected_geometry.as_ref().map_or(track.sample_rate,|g|g.0))?)
        } else {None};
        // Probe with the owned parser, rather than guessing from the sample
        // entry's channel count. Keep raw ASC and packet timestamps unchanged.
        let candidate = if track.codec == *b"mp4a" {
            let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
            let config = crate::codec::config::AudioSpecificConfig::parse(asc)?;
            if config.ps_present.is_none() && config.sbr_present != Some(false) {
                crate::codec::aac_ps_native::InBandPsProbe::new(asc, track.sample_rate).ok()
            } else { None }
        } else { None };
        let count = track.samples.len();
        let mut in_band_ps = false;
        if let Some(mut probe) = candidate {
            let mut payload = Vec::new();
            for sample in 0..count {
                demuxer.read_packet(index, sample, &mut payload)?;
                // A valid LC-only block has no SBR fill. It remains on normal
                // AAC dispatch; subsequent extension runs can still be probed.
                if probe.read(&payload).is_err() { probe.reset(); continue; }
                if probe.ps_detected() { in_band_ps = true; break; }
            }
        }
        let geometry=if let Some((_,channels,mask,layout))=&selected_geometry{Some((*channels,*mask,layout.clone()))}else{Self::dolby_geometry(&mut demuxer,index)?};
        let dolby_channels=geometry.as_ref().map(|(channels,_,_)|*channels);
        let dolby_mask=geometry.as_ref().and_then(|(_,mask,_)|*mask);
        let mut eac3_packet_layout=geometry.and_then(|(_,_,layout)|layout);
        if demuxer.tracks()[index].codec==*b"ec-3" {
            let description=demuxer.tracks()[index].samples.get(0).map_or(0,|s|s.description_index as usize-1);
            if let Ok(Some(layout))=Self::description_speakers(&mut demuxer,index,description,programme.unwrap_or(0)) {
                eac3_packet_layout=Some(layout);
            }
        }
        let mut dolby_track_channels=vec![None;demuxer.tracks().len()];
        dolby_track_channels[index]=dolby_channels;
        for other in Self::supported(&demuxer).into_iter().filter(|other|*other!=index) {
            // An unreadable/unmapped unselected track must not prevent opening
            // the selected valid track. Its original metadata remains visible.
            dolby_track_channels[other]=Self::dolby_geometry(&mut demuxer,other).ok().flatten().map(|(channels,_,_)|channels);
        }
        // A failed optional probe may have left an underlying reader between
        // packets. Always re-seek the selected first packet on real playback.
        demuxer.invalidate_position();
        Ok(Self {
            navigation_error:None,
            clock:None,clock_mode:std::cell::Cell::new(false),clock_floor:std::cell::Cell::new(EditTime::ZERO),clock_anchor:std::cell::Cell::new(None),
            variable_edits: None, opening_sample_index: 0,
            changing: false, active_description: 0, configuration_pending: false, description_states: Vec::new(),
            dolby_mask,
            dolby_channels,
            dolby_rate:selected_geometry.as_ref().map(|g|g.0),
            eac3_programme:programme.unwrap_or(0),
            eac3_stereo_output: false,
            eac3_output_layout: None,
            eac3_packet_layout,
            ac3_stereo_output: false,
            dolby_track_channels,
            in_band_ps,
            timeline,
            edit_cursor:std::cell::Cell::new(0),
            edit_segment:std::cell::Cell::new(None),
            demuxer,
            track_index: index,
            sample_index: 0,
            presentation_floor: 0,
            packet: Vec::new(),
        })
    }

    fn clock_point(&self,index:usize)->Result<EditTime> {
        if let Some(clock)=&self.clock {clock.point(self.track(),index)}else{EditTime::signed_ticks(self.track().samples.get(index).ok_or_else(||invalid("missing MP4 source point"))?.pts,self.track().timescale)}
    }
    fn source_time(&self,stamp:i64)->Result<EditTime> {
        if self.clock_mode.get() && self.clock.is_some() {self.clock_point(usize::try_from(stamp).map_err(|_|invalid("negative MP4 decoder stamp"))?)}else{EditTime::signed_ticks(stamp,self.track().timescale)}
    }
    fn clock_target(&self)->Result<EditTime> {
        if let Some(timeline)=&self.variable_edits {
            let index=timeline.locate().ok_or_else(||invalid("MP4 seek outside edits"))?;
            let segment=timeline.segments[index];
            Ok(segment.media.ok_or_else(||invalid("silence has no decoder preroll"))?.add(timeline.cursor.get().sub(segment.start)?)?)
        } else {Ok(self.clock_floor.get())}
    }
    fn clock_landing(&self,wanted:EditTime)->Result<(usize,EditTime)> {
        let clock=self.clock.as_ref().ok_or_else(||invalid("missing MP4 recovered clock"))?;
        if wanted.cmp(clock.end())!=std::cmp::Ordering::Less {return Ok((clock.count(),clock.end()));}
        let mut index=clock.at_time(self.track(),wanted)?;
        loop {
            let start=clock.point(self.track(),index)?;let rate=clock.rate(index)?;
            let frames=clock.frames(self.track(),index)?;
            let first=wanted.sub(start)?.ceil_frames(rate)?.clamp(0,frames as i128) as usize;
            if first<frames {return Ok((index,start.add(EditTime::new(first as i128,u128::from(rate))?)?));}
            index+=1;if index==clock.count() {return Ok((index,clock.end()));}
        }
    }
    fn seek_recovered(&mut self,wanted:EditTime)->Result<i64> {
        let (index,landed)=self.clock_landing(wanted)?;
        let mut first=index;
        if self.track().samples.get(index).is_some() {
            let setup=self.track().description_for_sample(index).ok_or_else(||invalid("missing MP4 seek description"))?;
            if matches!(&setup.codec,b"mp4a"|b"ac-3"|b"ec-3"|b"MAC3"|b"MAC6") {
                first=(0..index).rev().find(|&i|self.track().description_for_sample(i)!=Some(setup)).map_or(0,|i|i+1);
            }
        }
        let returned=i64::try_from(landed.floor_ticks(self.track().timescale)?).map_err(|_|invalid("MP4 seek clock overflow"))?;
        self.rewind();self.sample_index=first;self.clock_floor.set(landed);self.presentation_floor=returned;self.clock_anchor.set(Some((returned,landed)));Ok(returned)
    }
    fn align_variable_seek(&self,mut position:EditTime)->Result<EditTime> {
        let timeline=self.variable_edits.as_ref().ok_or_else(||invalid("missing MP4 edit timeline"))?;
        loop {
            timeline.cursor.set(position);
            let Some(index)=timeline.locate()else{return Ok(position);};
            let segment=timeline.segments[index];
            if let Some(media)=segment.media {
                let wanted=media.add(position.sub(segment.start)?)?;
                let (_,landed)=self.clock_landing(wanted)?;
                let mapped=landed.add(segment.start)?.sub(media)?;
                if mapped.cmp(segment.end)!=std::cmp::Ordering::Less {position=segment.end;continue;}
                position=position.max(mapped);
            }
            return Ok(position);
        }
    }
    fn seek_exact(&mut self,wanted:EditTime)->Result<i64> {
        if self.variable_edits.is_none() {return self.seek_recovered(wanted);}
        self.rewind();
        let timeline=self.variable_edits.as_ref().unwrap();
        let position=if wanted.cmp(timeline.end)==std::cmp::Ordering::Greater {timeline.end}else{wanted};
        timeline.cursor.set(position);
        let position=timeline.locate().map_or(position,|i|position.max(timeline.minimum_starts[i]));
        let position=self.align_variable_seek(position)?;timeline.cursor.set(position);
        self.variable_edits.as_mut().unwrap().active=None;
        let returned=i64::try_from(position.floor_ticks(self.track().timescale)?).map_err(|_|invalid("MP4 seek clock overflow"))?;
        self.clock_anchor.set(Some((returned,position)));Ok(returned)
    }
    fn negotiated_rate(&self)->u32 {self.dolby_rate.unwrap_or(self.setup().sample_rate)}
    fn native_eac3_layout(&self) -> Option<crate::codec::eac3_decoder::presentation::SpeakerLayout> {
        if self.setup().codec != *b"ec-3" { return None; }
        // Optional invalid/unsupported metadata retains packet probing;
        // it must not make the separately negotiated stereo path unavailable.
        if let Some(layout)=&self.eac3_output_layout {return Some(layout.clone());}
        let declared=crate::codec::eac3_decoder::presentation::configuration_speakers(
            &self.setup().configuration,self.eac3_programme).ok().flatten();
        match (declared,self.eac3_packet_layout.as_ref()) {
            (Some(mut layout),Some(observed))=>{
                layout.0.extend_from_slice(&observed.0);layout.0.sort();layout.0.dedup();
                if layout.validate_output().is_ok() {Some(layout)}else{Some(observed.clone())}
            },
            (Some(layout),None)=>Some(layout),
            (None,observed)=>observed.cloned(),
        }
    }
    fn negotiated_channels(&self)->u16 {if self.in_band_ps || (self.eac3_stereo_output && self.setup().codec==*b"ec-3") || (self.ac3_stereo_output && self.setup().codec==*b"ac-3") {2}else if self.setup().codec==*b"ec-3" && let Some(l)=self.native_eac3_layout() {l.0.len()as u16}else{self.dolby_channels.unwrap_or(self.setup().channels)}}
    fn setup(&self) -> &crate::container::mp4::SampleDescription {
        &self.track().sample_descriptions[self.active_description]
    }
    fn from_changing(demuxer: Mp4Reader<R>, index: usize, programme: Option<u8>,clock:Option<PlaybackClock>) -> Result<Self> {
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"soun"
            || track
                .sample_descriptions
                .iter()
                .any(|d| !CODECS.contains(&d.codec))
        {
            return Err(unsupported(
                "selected MP4 track contains unsupported audio sample descriptions",
            ));
        }
        let variable_edits = if track.edits.is_empty() {None} else {
            Some(VariableTimeline::new(&track.edits,track.timescale,demuxer.movie_timescale())?)
        };
        if programme.is_some()
            && !track
                .sample_descriptions
                .iter()
                .any(|d| d.codec == *b"ec-3")
        {
            return Err(unsupported("programme selection requires an E-AC-3 track"));
        }
        let first = track
            .samples
            .get(0)
            .ok_or_else(|| invalid("empty MP4 audio track"))?
            .description_index as usize
            - 1;
        let mut states = Vec::new();
        states
            .try_reserve_exact(track.sample_descriptions.len())
            .map_err(|_| invalid("MP4 description state allocation failed"))?;
        states.resize(track.sample_descriptions.len(), None);
        let tracks = demuxer.tracks().len();
        let mut source = Self {
            navigation_error:None,
            clock,clock_mode:std::cell::Cell::new(false),clock_floor:std::cell::Cell::new(EditTime::ZERO),clock_anchor:std::cell::Cell::new(None),
            variable_edits, opening_sample_index: 0,
            changing: true,
            active_description: first,
            configuration_pending: false,
            description_states: states,
            demuxer,
            track_index: index,
            in_band_ps: false,
            dolby_mask: None,
            dolby_channels: None,
            dolby_rate: None,
            eac3_programme: programme.unwrap_or(0),
            eac3_stereo_output: false,
            eac3_output_layout: None,
            eac3_packet_layout: None,
            ac3_stereo_output: false,
            dolby_track_channels: vec![None; tracks],
            sample_index: 0,
            presentation_floor: 0,
            packet: Vec::new(),
            timeline: None,
            edit_cursor: std::cell::Cell::new(0),
            edit_segment: std::cell::Cell::new(None),
        };
        source.configure_description(first)?;
        if source.variable_edits.is_some() {source.plan_edit_frontiers()?;let selected=source.variable_edits.as_ref().unwrap().locate();source.activate_edit(selected)?;}
        source.opening_sample_index=source.sample_index;
        Ok(source)
    }
    fn configure_description(&mut self, selected: usize) -> Result<()> {
        let state = if let Some(state) = self.description_states[selected].clone() {
            state
        } else {
            let setup = self.track().sample_descriptions[selected].clone();
            let mut state = DescriptionState::default();
            let first = (0..self.track().samples.len())
                .find(|&i| {
                    self.track().samples.get(i).unwrap().description_index as usize == selected + 1
                })
                .ok_or_else(|| invalid("unused MP4 audio description selected"))?;
            if setup.codec == *b"mp4a" {
                let asc = crate::codec::config::aac_specific_config(&setup.configuration)?;
                let config = crate::codec::config::AudioSpecificConfig::parse(asc)?;
                if config.ps_present.is_none()
                    && config.sbr_present != Some(false)
                    && let Ok(mut probe) =
                        crate::codec::aac_ps_native::InBandPsProbe::new(asc, setup.sample_rate)
                {
                    let mut payload = Vec::new();
                    for i in first..self.track().samples.len() {
                        if self.track().samples.get(i).unwrap().description_index as usize
                            != selected + 1
                        {
                            probe.reset();
                            continue;
                        }
                        self.demuxer
                            .read_packet(self.track_index, i, &mut payload)?;
                        if probe.read(&payload).is_err() {
                            probe.reset();
                            continue;
                        }
                        if probe.ps_detected() {
                            state.in_band_ps = true;
                            break;
                        }
                    }
                }
            } else if setup.codec == *b"ac-3" {
                let mut payload = Vec::new();
                let end=(first+1..self.track().samples.len())
                    .find(|&i|self.track().description_for_sample(i)!=Some(&setup))
                    .unwrap_or(self.track().samples.len());
                let mut valid=None;
                for i in first..end {
                    self.demuxer.read_packet(self.track_index,i,&mut payload)?;
                    let mut tail=payload.as_slice();
                    while let Some(header)=crate::codec::ac3_decoder::syncframe(tail) {
                        let Some(encoded)=tail.get(..header.frame_bytes)else{break;};
                        if let Some(frame)=crate::codec::ac3_decoder::checked_syncframe(encoded) {
                            let mask=crate::codec::ac3_decoder::speaker_mask(encoded,frame.channels)?;
                            valid=Some((frame,mask));break;
                        }
                        tail=&tail[header.frame_bytes..];
                    }
                    if valid.is_some(){break;}
                }
                let (frame,mask)=valid.ok_or_else(||invalid("invalid AC-3 description setup"))?;
                state.rate = Some(frame.sample_rate);
                state.channels = Some(frame.channels);
                state.mask = Some(mask);
            } else if setup.codec == *b"ec-3" {
                const FRAMES: usize = 2 * 8 * 9;
                const BYTES: usize = FRAMES * 4096;
                let end = (first + 1..self.track().samples.len())
                    .find(|&i| {
                        self.track().samples.get(i).unwrap().description_index as usize
                            != selected + 1
                    })
                    .unwrap_or(self.track().samples.len());
                let mut prefix = Vec::new();
                let mut payload = Vec::new();
                let mut found = None;
                for i in first..end.min(first + FRAMES) {
                    self.demuxer
                        .read_packet(self.track_index, i, &mut payload)?;
                    prefix.extend_from_slice(
                        &payload[..payload.len().min(BYTES.saturating_sub(prefix.len()))],
                    );
                    if let Some((frame, layout, dependent)) =
                        crate::codec::eac3_decoder::presentation::probe_native_programme(
                            &prefix,
                            self.eac3_programme,
                            i + 1 == end,
                        )?
                    {
                        state.native_layout=dependent.then(||layout.clone());
                        found = Some((
                            frame.sample_rate,
                            layout.0.len() as u16,
                            layout.wave_mask(),
                        ));
                        break;
                    }
                    if prefix.len() == BYTES {
                        break;
                    }
                }
                let (rate, channels, mask) = found.ok_or_else(|| {
                    invalid("selected E-AC-3 programme is missing or incomplete in setup")
                })?;
                state.rate = Some(rate);
                state.channels = Some(channels);
                state.mask = mask;
                if let Ok(Some(layout))=Self::description_speakers(&mut self.demuxer,self.track_index,selected,self.eac3_programme) {
                    state.native_layout=Some(layout);
                }
            }
            self.description_states[selected] = Some(state.clone());
            state
        };
        self.active_description = selected;
        self.configuration_pending = false;
        self.in_band_ps = state.in_band_ps;
        self.dolby_rate = state.rate;
        self.dolby_channels = state.channels;
        self.dolby_mask = state.mask;
        self.eac3_packet_layout=state.native_layout.clone();
        self.dolby_track_channels[self.track_index] = state.channels;
        self.demuxer.invalidate_position();
        Ok(())
    }
    fn plan_edit_frontiers(&mut self) -> Result<()> {
        let count=self.variable_edits.as_ref().unwrap().segments.len();
        let mut starts=Vec::new();starts.try_reserve_exact(count).map_err(|_|invalid("MP4 edit frontier allocation failed"))?;
        let mut cursor=EditTime::ZERO;
        for index in 0..count {
            let segment=self.variable_edits.as_ref().unwrap().segments[index];
            let begin=cursor.max(segment.start);starts.push(begin);
            if begin.cmp(segment.end)!=std::cmp::Ordering::Less {continue;}
            if let Some(media)=segment.media {
                let media_end=media.add(segment.end.sub(segment.start)?)?;
                let floor=media_end.floor_ticks(self.track().timescale)?;
                let exact=EditTime::new(floor,u128::from(self.track().timescale))?==media_end;
                let tick=(floor-i128::from(exact)).clamp(i128::from(i64::MIN),i128::from(i64::MAX)) as i64;
                let frontier=if let Some(clock)=&self.clock {clock.at_time(self.track(),media_end)?}else{self.track().samples.at_or_before(tick)};
                let frontier=if frontier>0 && self.clock_point(frontier)?.cmp(media_end)!=std::cmp::Ordering::Less {frontier-1}else{frontier};
                let sample=self.track().samples.get(frontier).ok_or_else(||invalid("missing MP4 edit frontier sample"))?;
                self.configure_description(sample.description_index as usize-1)?;
                let rate=self.negotiated_rate();
                let native_frames=if let Some(clock)=&self.clock {clock.frames(self.track(),frontier)? as i128}else if matches!(&self.setup().codec,b"sowt"|b"twos"|b"raw "|b"in24"|b"in32"|b"fl32"|b"fl64") {
                    let bits=usize::from(self.setup().bit_depth);let channels=usize::from(self.setup().channels);
                    let stride=bits.div_ceil(8).checked_mul(channels).filter(|n|*n>0).ok_or_else(||invalid("invalid PCM edit frontier geometry"))?;
                    if !(sample.size as usize).is_multiple_of(stride) {return Err(invalid("unaligned PCM edit frontier"));}
                    i128::from(sample.size)/stride as i128
                } else { i128::from(sample.duration)*i128::from(rate)/i128::from(self.track().timescale) };
                let sample_index=frontier;
                let mapped=self.clock_point(sample_index)?.add(segment.start)?.sub(media)?;
                let first=begin.sub(mapped)?.ceil_frames(rate)?.clamp(0,native_frames);
                let last=segment.end.sub(mapped)?.ceil_frames(rate)?.clamp(0,native_frames);
                if last>first {cursor=mapped.add(EditTime::new(last,u128::from(rate))?)?;}
            } else {
                let initial=self.track().samples.get(0).unwrap().description_index as usize-1;
                self.configure_description(initial)?;let rate=self.negotiated_rate();
                let first=begin.sub(segment.start)?.ceil_frames(rate)?;
                let last=segment.end.sub(segment.start)?.ceil_frames(rate)?;
                if last>first {cursor=segment.start.add(EditTime::new(last,u128::from(rate))?)?;}
            }
        }
        self.variable_edits.as_mut().unwrap().minimum_starts=starts;
        Ok(())
    }
    fn edit_source_index(&self, segment: edit_clock::Segment, cursor: EditTime) -> Result<usize> {
        let Some(media)=segment.media else {return Ok(0);};
        let wanted=media.add(cursor.max(segment.start).sub(segment.start)?)?;
        let ticks=wanted.floor_ticks(self.track().timescale)?.clamp(0,i128::from(i64::MAX)) as i64;
        let selected=if let Some(clock)=&self.clock {clock.at_time(self.track(),wanted)?}else{self.track().samples.at_or_before(ticks)};
        let setup=self.track().description_for_sample(selected).ok_or_else(||invalid("missing MP4 edit source sample"))?;
        if matches!(&setup.codec,b"mp4a"|b"ac-3"|b"ec-3"|b"MAC3"|b"MAC6") {
            // Recover history from the last real setup boundary, including
            // equivalent table entries that deliberately preserve the decoder.
            Ok((0..selected).rev().find(|&i|self.track().description_for_sample(i)!=Some(setup)).map_or(0,|i|i+1))
        } else {Ok(selected)}
    }
    fn activate_edit(&mut self, mut selected: Option<usize>) -> Result<()> {
        while let Some(index)=selected {
            let timeline=self.variable_edits.as_ref().unwrap();
            timeline.cursor.set(timeline.cursor.get().max(timeline.minimum_starts[index]));
            let resolved=timeline.locate();
            if resolved==selected {break;}selected=resolved;
        }
        if let Some(index)=selected {
            let timeline=self.variable_edits.as_ref().unwrap();let segment=timeline.segments[index];
            let source=self.edit_source_index(segment,timeline.cursor.get())?;
            self.sample_index=source;
            let setup=self.track().samples.get(source).ok_or_else(||invalid("empty MP4 audio edit source"))?.description_index as usize-1;
            self.configure_description(setup)?;
        }
        self.variable_edits.as_mut().unwrap().active=selected;
        self.configuration_pending=false;
        Ok(())
    }
    fn next_variable_step(&mut self) -> Result<EditAction> {
        loop {
            let timeline=self.variable_edits.as_ref().unwrap();
            let Some(index)=timeline.locate() else {return Ok(EditAction::End);};
            if timeline.active!=Some(index) {self.configuration_pending=true;return Ok(EditAction::Step(crate::audio::AudioStep::ReconfigureDecoder));}
            let segment=timeline.segments[index];
            if segment.media.is_none() {
                let rate=self.negotiated_rate();if rate==0{return Err(invalid("zero MP4 silence rate"));}
                let first=timeline.cursor.get().max(segment.start).sub(segment.start)?.ceil_frames(rate)?;
                let start=segment.start.add(EditTime::new(first,u128::from(rate))?)?;
                let frames=(segment.end.sub(segment.start)?.ceil_frames(rate)?-first).min(1024).max(0) as usize;
                if frames==0 {timeline.cursor.set(segment.end);continue;}
                let stride=usize::from(self.negotiated_channels()).checked_mul(4).filter(|n|*n>0).ok_or_else(||invalid("invalid MP4 silence geometry"))?;
                let data=crate::buffer(frames.checked_mul(stride).ok_or_else(||invalid("MP4 silence allocation overflow"))?)?;
                let (pts,den)=start.stamp()?;
                timeline.cursor.set(start.add(EditTime::ticks(frames as u64,rate)?)?);
                return Ok(EditAction::Step(crate::audio::AudioStep::Pcm(crate::audio::AudioPacket {data,pts,timebase_num:1,timebase_den:den})));
            }
            if self.sample_index==self.track().samples.len() {
                // A delayed decoder may still complete this edit. Its tail must
                // be clipped using this segment before deciding whether to repeat.
                self.configuration_pending=true;
                return Ok(EditAction::Step(crate::audio::AudioStep::ReconfigureDecoder));
            }
            return Ok(EditAction::Encoded);
        }
    }
    fn present_variable_edit(&self, mut packet:crate::audio::AudioPacket, source_pts:i64) -> Result<Option<crate::audio::AudioPacket>> {
        let timeline=self.variable_edits.as_ref().unwrap();
        let Some(index)=timeline.active else {return Ok(None);};let segment=timeline.segments[index];
        let Some(media)=segment.media else {return Ok(None);};
        if timeline.cursor.get().cmp(segment.end)!=std::cmp::Ordering::Less {return Ok(None);}
        let rate=self.negotiated_rate();let stride=usize::from(self.negotiated_channels()).checked_mul(4).filter(|n|*n>0).ok_or_else(||invalid("invalid MP4 edit PCM stride"))?;
        if rate==0 || !packet.data.len().is_multiple_of(stride) {return Err(invalid("invalid MP4 edit PCM geometry"));}
        let frames=packet.data.len()/stride;
        let mapped=self.source_time(source_pts)?.add(segment.start)?.sub(media)?;
        let first=timeline.cursor.get().max(segment.start).sub(mapped)?.ceil_frames(rate)?.clamp(0,frames as i128) as usize;
        let last=segment.end.sub(mapped)?.ceil_frames(rate)?.clamp(0,frames as i128) as usize;
        if first>=last {
            let end=mapped.add(EditTime::ticks(last as u64,rate)?)?;
            if mapped.cmp(segment.end)==std::cmp::Ordering::Less && end.cmp(segment.end)!=std::cmp::Ordering::Less {
                timeline.cursor.set(timeline.cursor.get().max(end));
            }
            return Ok(None);
        }
        let start=mapped.add(EditTime::ticks(first as u64,rate)?)?;
        let end=mapped.add(EditTime::ticks(last as u64,rate)?)?;
        packet.data.copy_within(first*stride..last*stride,0);packet.data.truncate((last-first)*stride);
        let (pts,den)=start.stamp()?;packet.pts=pts;packet.timebase_num=1;packet.timebase_den=den;
        timeline.cursor.set(end);
        Ok(Some(packet))
    }
    /// Read the next audio packet. Returns None at end of track.
    /// Changing setups require `AudioStream::next_step` so the old decoder can
    /// drain before the source commits the next configuration.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        let track_index = self.track_index;
        let sample_index = self.sample_index;
        if self.changing && self.track().samples.get(sample_index).is_some_and(|s|s.description_index as usize!=self.active_description+1) {
            return Err(unsupported("changing MP4 audio descriptions require AudioStream::next_step"));
        }
        let track = self.track();
        let sample = match track.samples.get(sample_index) {
            Some(s) => s,
            None => return Ok(None),
        };
        let pts = sample.pts;
        let duration = sample.duration as i64;

        self.demuxer
            .read_packet(track_index, sample_index, &mut self.packet)?;

        let packet = AudioPacket {
            data: self.packet.clone(),
            pts,
            duration,
            sample_index,
        };

        self.sample_index += 1;
        Ok(Some(packet))
    }

    /// The audio track metadata.
    pub fn track(&self) -> &Track {
        &self.demuxer.tracks()[self.track_index]
    }

    /// Restart from the first sample.
    pub fn rewind(&mut self) {
        self.demuxer.invalidate_position();
        self.sample_index = if self.variable_edits.is_some(){self.opening_sample_index}else{0};
        self.configuration_pending = false;
        if let Some(timeline)=&mut self.variable_edits {timeline.cursor.set(EditTime::ZERO);timeline.active=None;}
        if self.changing {
            let initial=self.track().samples.get(self.sample_index).unwrap().description_index as usize-1;
            // The opening configuration was negotiated before exposing the reader.
            let state=self.description_states[initial].clone().unwrap();self.active_description=initial;
            self.in_band_ps=state.in_band_ps;self.dolby_rate=state.rate;self.dolby_channels=state.channels;self.dolby_mask=state.mask;self.eac3_packet_layout=state.native_layout;
        }
        self.presentation_floor = 0;self.navigation_error=None;
        self.clock_floor.set(EditTime::ZERO);self.clock_anchor.set(None);
        self.packet.clear();
        self.edit_cursor.set(0);
        self.edit_segment.set(None);
    }

    /// Seek to the sample with the greatest PTS at or before `pts`.
    pub fn seek(&mut self, pts: i64) -> i64 {
        let track = self.track();
        let index = track.samples.at_or_before(pts);
        let result_pts = track.samples.get(index).map_or(0, |s| s.pts);
        self.rewind();
        self.sample_index = index;
        result_pts
    }
}

/// Packets are timestamped in the *track* timescale, which differs from the
/// movie timescale in most files (sample rate versus a nominal 1000 Hz).
impl<R: Read + Seek + Send> crate::audio::AudioStream for Mp4AudioReader<R> {
    fn decoder_stamp(&self,packet:&crate::audio::EncodedPacket)->i64 {
        if self.clock.is_some() {self.clock_mode.set(true);self.sample_index.saturating_sub(1) as i64}else{packet.pts}
    }
    fn checkpoint_bucket(&self,stamp:i64)->i64 {
        if self.clock_mode.get() && self.clock.is_some() {
            return self.source_time(stamp).and_then(|t|t.floor_ticks(1)).ok().and_then(|n|i64::try_from(n).ok()).unwrap_or(stamp);
        }
        stamp/i64::from(self.track().timescale.max(1))
    }
    fn decoded_packet_sample_limit(&self,stamp:i64,duration:u64)->Result<Option<usize>> {
        if self.clock_mode.get() && let Some(clock)=&self.clock {
            return clock.frames(self.track(),usize::try_from(stamp).map_err(|_|invalid("negative MP4 decoder stamp"))?).map(Some);
        }
        self.packet_sample_limit(duration)
    }
    fn set_eac3_output_layout(&mut self, layout:Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>)->Result<()> {
        if let Some(l)=&layout {if self.setup().codec!=*b"ec-3" {return Err(invalid("native E-AC-3 layout requires an E-AC-3 source"));}l.validate_output()?;}
        self.eac3_output_layout=layout;Ok(())
    }
    fn set_ac3_stereo_output(&mut self, enabled: bool) -> Result<()> {self.ac3_stereo_output=enabled;Ok(())}
    fn set_eac3_stereo_output(&mut self, enabled: bool) -> Result<()> {
        self.eac3_stereo_output=enabled;
        Ok(())
    }
    fn speaker_mask(&self)->Result<Option<u32>> {
        if (self.eac3_stereo_output && self.setup().codec==*b"ec-3") || (self.ac3_stereo_output && self.setup().codec==*b"ac-3") {return Ok(Some(3));}
        if self.setup().codec==*b"ec-3" && let Some(l)=self.native_eac3_layout() {return Ok(l.wave_mask());}
        if matches!(&self.setup().codec,b"ac-3"|b"ec-3") {return Ok(self.dolby_mask);}
        if self.setup().codec==*b"alac" {crate::codec::alac_decoder::AlacDecoder::configuration_mask(&self.setup().configuration,self.sample_rate(),self.setup().channels).map(Some)}else{Ok(None)}
    }

    fn stereo_router(&self)->Result<Option<fvid_media::owned_stereo_router::StereoRouter>>{
        if self.setup().codec==*b"ec-3" && self.channels()>6 && self.speaker_mask()?.is_none() {return Err(unsupported("E-AC-3 speaker locations need an extended speaker mask"));}
        if self.setup().codec!=*b"mp4a"{return self.speaker_mask()?.map(|mask|fvid_media::owned_stereo_router::StereoRouter::new(self.channels(),mask).map_err(|e|invalid(&e))).transpose();}
        let asc=crate::codec::config::aac_specific_config(&self.setup().configuration)?;
        fvid_media::owned_aac::speaker_routing::stereo_router(asc,self.sample_rate(),self.channels(),self.in_band_ps).map(Some).map_err(|e|invalid(&e))
    }

    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        if self.setup().codec == *b"ac-3" {
            if self.ac3_stereo_output {return Ok(Box::new(crate::codec::ac3_decoder::Ac3Decoder::for_container_stereo(self.extra_data(),self.sample_rate())?));}
            return Ok(Box::new(crate::codec::ac3_decoder::Ac3Decoder::for_container(self.extra_data(),self.sample_rate(),self.channels())?));
        }
        if self.setup().codec == *b"ec-3" {
            if self.eac3_stereo_output {
                return Ok(Box::new(crate::codec::eac3_decoder::Eac3Decoder::for_container_stereo(self.extra_data(),self.sample_rate())?.with_programme(self.eac3_programme)?));
            }
            let mut d=Box::new(crate::codec::eac3_decoder::Eac3Decoder::for_container(self.extra_data(),self.sample_rate(),self.channels())?.with_programme(self.eac3_programme)?);
            if let Some(layout)=self.native_eac3_layout() {d.set_output_layout(layout)?;}
            return Ok(d);
        }
        if self.in_band_ps {
            Ok(Box::new(crate::codec::aac_ps_playback::PsAacDecoder::new_with_in_band_ps(
                &self.setup().configuration, self.sample_rate(), 2,
            )?))
        } else {
            let mut decoder=crate::codec::make_audio_decoder(self.codec(), self.extra_data(), self.sample_rate(), self.channels(), self.bits_per_sample())?;
            decoder.set_source_timescale(self.track().timescale)?;
            if self.clock.as_ref().is_some_and(|c|c.full_packet(self.sample_index)) {decoder.defer_packet_trimming();}
            Ok(decoder)
        }
    }

    fn codec(&self) -> &str {
        let track = self.setup();
        match codec_name(&track.codec) {
            Some(name) => name,
            None => std::str::from_utf8(&track.codec).unwrap_or(""),
        }
    }

    fn timescale(&self) -> u32 {
        self.track().timescale
    }

    fn sample_rate(&self) -> u32 {
        self.dolby_rate.unwrap_or(self.setup().sample_rate)
    }

    fn channels(&self) -> u16 {
        self.negotiated_channels()
    }

    fn bits_per_sample(&self) -> u16 {
        self.setup().bit_depth
    }

    fn preroll_target(&self)->Option<i64> {
        if self.clock_mode.get() && let Some(clock)=&self.clock {
            if self.setup().codec!=*b"mp4a" {return None;}
            return self.clock_target().ok().and_then(|t|clock.at_time(self.track(),t).ok()).and_then(|n|i64::try_from(n).ok());
        }
        if self.changing || self.setup().codec!=*b"mp4a" {return None;}
        if let Some(t)=&self.timeline {
            let (_,source)=t.locate(self.edit_cursor.get())?;
            let ticks=u128::from(source?)*u128::from(self.track().timescale)/u128::from(self.sample_rate());
            i64::try_from(ticks).ok()
        } else {Some(self.presentation_floor)}
    }
    fn resume_preroll(&mut self,pts:i64)->bool {
        if self.clock_mode.get() && let Some(clock)=&self.clock {
            let Ok(index)=usize::try_from(pts)else{return false;};
            let Some(target)=self.preroll_target().and_then(|n|usize::try_from(n).ok())else{return false;};
            if index>target || !clock.same_run(index,target) || self.track().description_for_sample(index)!=Some(self.setup()) {return false;}
            self.sample_index=index;self.demuxer.invalidate_position();return true;
        }
        if self.preroll_target().is_none_or(|target|pts>target) {return false;}
        let index=self.track().samples.at_or_before(pts);
        if self.track().samples.get(index).is_none_or(|s|s.pts!=pts) {return false;}
        self.sample_index=index;true
    }
    fn duration(&self) -> Option<std::time::Duration> {
        if let Some(timeline)=&self.variable_edits {return timeline.end.duration();}
        if let Some(clock)=&self.clock {return clock.end().duration();}
        let track = self.track();
        if (track.codec == *b"mp4a" || self.timeline.is_some()) && !track.edits.is_empty() && self.demuxer.movie_timescale() != 0 {
            let ticks = track.edits.iter().try_fold(0u128, |sum, edit| sum.checked_add(u128::from(edit.duration)))?;
            let nanos = ticks.checked_mul(1_000_000_000)? / u128::from(self.demuxer.movie_timescale());
            return Some(std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)));
        }
        (track.timescale > 0 && track.duration > 0).then(|| {
            let nanos = u128::from(track.duration) * 1_000_000_000 / u128::from(track.timescale);
            std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
        })
    }

    fn time_of(&self, pts: i64) -> std::time::Duration {
        if let Some((stamp,time))=self.clock_anchor.get() && stamp==pts {return time.duration().unwrap_or_default();}
        let track = self.track();
        let shift = if self.variable_edits.is_none() && track.codec == *b"mp4a" && track.edits.len() == 1 && track.edits[0].media_time >= 0 { track.edits[0].media_time } else { 0 };
        if track.timescale == 0 { return std::time::Duration::ZERO; }
        let nanos = pts.saturating_sub(shift).max(0) as u128 * 1_000_000_000 / u128::from(track.timescale);
        std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    fn packet_sample_limit(&self, duration: u64) -> Result<Option<usize>> {
        if self.setup().codec != *b"mp4a" { return Ok(None); }
        let rate = u128::from(self.sample_rate());
        let scale = u128::from(self.track().timescale);
        let samples = u128::from(duration) * rate;
        if scale == 0 || samples == 0 || samples % scale != 0 {
            return Err(invalid("MP4 AAC packet duration is not sample aligned"));
        }
        Ok(Some(usize::try_from(samples / scale).map_err(|_| invalid("MP4 AAC sample window overflow"))?))
    }

    fn present_decoded(&self, mut packet: crate::audio::AudioPacket, source_pts: i64) -> Result<Option<crate::audio::AudioPacket>> {
        if self.variable_edits.is_some() {return self.present_variable_edit(packet,source_pts);}
        if self.clock_mode.get() && let Some(clock)=&self.clock {
            let start=self.source_time(source_pts)?;let rate=self.sample_rate();let stride=usize::from(self.channels())*4;
            if rate==0 || stride==0 || !packet.data.len().is_multiple_of(stride) {return Err(invalid("invalid recovered MP4 PCM geometry"));}
            let frames=packet.data.len()/stride;
            let first=self.clock_floor.get().max(EditTime::ZERO).sub(start)?.ceil_frames(rate)?.clamp(0,frames as i128) as usize;
            let last=clock.end().sub(start)?.ceil_frames(rate)?.clamp(0,frames as i128) as usize;
            if first>=last {return Ok(None);}
            packet.data.copy_within(first*stride..last*stride,0);packet.data.truncate((last-first)*stride);
            let at=start.add(EditTime::new(first as i128,u128::from(rate))?)?;let (pts,den)=at.stamp()?;
            packet.pts=pts;packet.timebase_num=1;packet.timebase_den=den;return Ok(Some(packet));
        }
        let track = self.track();
        if let Some(timeline) = &self.timeline {
            let cursor=self.edit_cursor.get();
            let (index,_) = timeline.locate(cursor).ok_or_else(||invalid("AAC presentation cursor outside edits"))?;
            let segment=&timeline.segments[index];
            let source=segment.source_start.ok_or_else(||invalid("decoded AAC inside silence edit"))?;
            let rate=u128::from(self.sample_rate());let scale=u128::from(track.timescale);
            let start=(source_pts.max(0) as u128*rate/scale) as u64;
            let stride=usize::from(self.channels())*4;
            if stride==0 || !packet.data.len().is_multiple_of(stride) {return Err(invalid("invalid AAC PCM stride"));}
            let frames=packet.data.len()/stride;
            let wanted=source.checked_add(cursor-segment.presentation.start).ok_or_else(||invalid("AAC edit source overflow"))?;
            let source_end=source.checked_add(segment.presentation.end-segment.presentation.start).ok_or_else(||invalid("AAC edit source overflow"))?;
            let first=wanted.saturating_sub(start).min(frames as u64) as usize;
            let last=source_end.saturating_sub(start).min(frames as u64) as usize;
            if first>=last {return Ok(None);}
            let pts=segment.presentation.start+(start+first as u64-source);
            if pts!=cursor {return Err(invalid("AAC edit source gap"));}
            packet.data.copy_within(first*stride..last*stride,0);packet.data.truncate((last-first)*stride);
            packet.pts=pts;packet.timebase_num=1;packet.timebase_den=self.sample_rate();
            self.edit_cursor.set(pts+(last-first) as u64);
            return Ok(Some(packet));
        }
        if self.changing || track.codec != *b"mp4a" {
            let rate = self.sample_rate();
            let scale = track.timescale;
            let stride = usize::from(self.channels()) * 4;
            if rate == 0 || scale == 0 || stride == 0 || !packet.data.len().is_multiple_of(stride) {
                return Err(invalid("invalid MP4 decoded audio geometry"));
            }
            let frames = packet.data.len() / stride;
            if frames == 0 { return Ok(None); }
            if source_pts < self.presentation_floor {
                let skip = (((i128::from(self.presentation_floor) - i128::from(source_pts)) as u128 * u128::from(rate))
                    .div_ceil(u128::from(scale))).min(frames as u128) as usize;
                if skip == frames { return Ok(None); }
                packet.data.copy_within(skip * stride.., 0);
                packet.data.truncate((frames - skip) * stride);
                let numerator = i128::from(source_pts) * i128::from(rate)
                    + skip as i128 * i128::from(scale);
                let at = numerator * 1_000_000_000 / (i128::from(rate) * i128::from(scale));
                packet.pts = u64::try_from(at).map_err(|_| invalid("negative MP4 audio presentation time"))?;
                packet.timebase_den = 1_000_000_000;
            } else {
                // Decoder output labels the integer PTS with a sample clock;
                // the caller supplied track media units, which need not match.
                packet.pts = source_pts as u64;
                packet.timebase_den = scale;
            }
            packet.timebase_num = 1;
            return Ok(Some(packet));
        }
        // Single media edits cover encoder priming and padding. Complex edit
        // sequences require replay/silence scheduling beyond this packet hook.
        if track.codec != *b"mp4a" || track.edits.len() > 1 || track.edits.first().is_some_and(|e| e.media_time < 0) {
            return Ok(Some(packet));
        }
        let rate = u128::from(self.sample_rate());
        let scale = u128::from(track.timescale);
        let movie_scale = u128::from(self.demuxer.movie_timescale());
        if scale == 0 || movie_scale == 0 { return Err(invalid("zero MP4 audio presentation clock")); }
        let (media_start, length) = if let Some(edit) = track.edits.first() {
            ((edit.media_time as u128 * rate).div_ceil(scale), (u128::from(edit.duration) * rate).div_ceil(movie_scale))
        } else { (0, (u128::from(track.duration) * rate).div_ceil(scale)) };
        let media_end = media_start.checked_add(length).ok_or_else(|| invalid("MP4 audio edit overflow"))?;
        let source_start = i128::from(source_pts) * i128::from(self.sample_rate()) / i128::from(track.timescale);
        let stride = usize::from(self.channels()).checked_mul(4).filter(|n| *n != 0).ok_or_else(|| invalid("invalid AAC PCM stride"))?;
        if !packet.data.len().is_multiple_of(stride) { return Err(invalid("unaligned AAC PCM buffer")); }
        let frames = packet.data.len()/stride;
        let floor = (self.presentation_floor.max(0) as u128 * rate).div_ceil(scale).max(media_start);
        let first = (i128::try_from(floor).map_err(|_| invalid("audio edit clock overflow"))?-source_start).clamp(0,frames as i128) as usize;
        let last = (i128::try_from(media_end).map_err(|_| invalid("audio edit clock overflow"))?-source_start).clamp(0,frames as i128) as usize;
        if first >= last { return Ok(None); }
        packet.data.copy_within(first*stride..last*stride,0);
        packet.data.truncate((last-first)*stride);
        packet.pts = u64::try_from(source_start+first as i128-media_start as i128).map_err(|_| invalid("negative audio presentation time"))?;
        packet.timebase_num = 1;
        packet.timebase_den = self.sample_rate();
        Ok(Some(packet))
    }

    fn extra_data(&self) -> &[u8] {
        &self.setup().configuration
    }

    fn audio_track_index(&self) -> usize { Self::supported(&self.demuxer).iter().position(|&index|index==self.track_index).unwrap_or(0) }
    fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
        Mp4AudioReader::supported(&self.demuxer)
            .into_iter()
            .map(|index| {
                let track = &self.demuxer.tracks()[index];
                crate::audio::AudioTrack {
                    sample_rate: if index==self.track_index{self.sample_rate()}else{track.sample_rate},
                    channels: if index==self.track_index {self.channels()}else{self.dolby_track_channels[index].unwrap_or(track.channels)},
                    name: track.name.clone(),
                    language: track.language.clone(),
                }
            })
            .collect()
    }

    fn next_packet(&mut self) -> Result<Option<crate::audio::EncodedPacket>> {
        Ok(
            Mp4AudioReader::read_packet(self)?.map(|p| crate::audio::EncodedPacket {
                data: p.data,
                pts: p.pts,
                duration: p.duration,
            }),
        )
    }

    fn drain_decoded_at_eof(&self)->bool {
        if self.variable_edits.is_some() {return false;}
        self.timeline.as_ref().is_none_or(|timeline| timeline.locate(self.edit_cursor.get()).is_some_and(|(_,source)|source.is_some()))
    }
    fn validate_eof(&self)->Result<()> {
        if self.variable_edits.as_ref().is_some_and(|t|t.locate().is_some()) {return Err(invalid("MP4 audio edit extends beyond source packets"));}
        if self.timeline.as_ref().is_some_and(|timeline|timeline.locate(self.edit_cursor.get()).is_some()) {
            return Err(invalid("AAC edit extends beyond source packets"));
        }
        Ok(())
    }
    fn advance_configuration(&mut self) -> Result<()> {
        if !self.changing || !self.configuration_pending {return Err(invalid("no pending MP4 audio configuration"));}
        if let Some(timeline)=&self.variable_edits {
            let selected=timeline.locate();
            if selected!=timeline.active {return self.activate_edit(selected);}
            if self.sample_index==self.track().samples.len() {return Err(invalid("MP4 audio edit extends beyond source packets"));}
        }
        let selected=self.track().samples.get(self.sample_index).ok_or_else(||invalid("missing MP4 audio configuration sample"))?.description_index as usize-1;
        self.configure_description(selected)
    }
    fn reset_decoder(&mut self, decoder:&mut Box<dyn crate::audio::AudioDecode>) -> Result<()> {
        if self.changing {
            if let Some(timeline)=&self.variable_edits {
                let selected=timeline.locate();
                if selected!=timeline.active || timeline.active.is_none() {self.activate_edit(selected)?;}
            }
            let selected=self.track().samples.get(self.sample_index).or_else(||self.track().samples.get(0)).unwrap().description_index as usize-1;
            self.configure_description(selected)?;*decoder=self.make_decoder()?;
        } else {decoder.reset();}
        Ok(())
    }
    fn next_step(&mut self)->Result<Option<crate::audio::AudioStep>> {
        use crate::audio::{AudioStep,AudioPacket};
        if let Some(error)=self.navigation_error.take() {return Err(error);}
        if self.variable_edits.is_some() {match self.next_variable_step()? {EditAction::End=>return Ok(None),EditAction::Step(step)=>return Ok(Some(step)),EditAction::Encoded=>{}}}
        if self.changing && let Some(sample)=self.track().samples.get(self.sample_index) {
            let selected=sample.description_index as usize-1;
            if selected!=self.active_description {
                if self.track().sample_descriptions[selected]==*self.setup() {
                    self.description_states[selected]=self.description_states[self.active_description].clone();self.active_description=selected;
                } else {self.configuration_pending=true;return Ok(Some(AudioStep::ReconfigureDecoder));}
            }
        }
        let Some(timeline)=&self.timeline else {return self.next_packet().map(|p|p.map(AudioStep::Encoded));};
        let cursor=self.edit_cursor.get();
        let Some((index,source))=timeline.locate(cursor) else {return Ok(None);};
        if self.edit_segment.get()!=Some(index) {
            self.edit_segment.set(Some(index));self.sample_index=0;
            if source.is_some() {return Ok(Some(AudioStep::ResetDecoder));}
        }
        if source.is_none() {
            let frames=(timeline.segments[index].presentation.end-cursor).min(1024);
            let stride=usize::from(self.channels())*4;
            let data=crate::buffer(frames as usize*stride)?;
            self.edit_cursor.set(cursor+frames);
            return Ok(Some(AudioStep::Pcm(AudioPacket {data,pts:cursor,timebase_num:1,timebase_den:self.sample_rate()})));
        }
        Ok(self.next_packet()?.map(AudioStep::Encoded))
    }

    fn rewind(&mut self) {
        Mp4AudioReader::rewind(self);
    }

    fn seek_time(&mut self,target:std::time::Duration)->i64 {
        if self.clock.is_some() {
            let time=EditTime::new(target.as_nanos() as i128,1_000_000_000).and_then(|t|self.seek_exact(t));
            return match time {Ok(at)=>at,Err(error)=>{self.navigation_error=Some(error);0}};
        }
        let scale=u128::from(self.track().timescale);let mut ticks=target.as_nanos()*scale/1_000_000_000;
        if scale>0 && scale<1_000_000_000 && (ticks+1)*1_000_000_000/scale==target.as_nanos(){ticks+=1;}
        self.seek_to(i64::try_from(ticks).unwrap_or(i64::MAX))
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        if self.clock.is_some() {
            let result=EditTime::signed_ticks(pts.max(0),self.track().timescale).and_then(|t|self.seek_exact(t));
            return match result {Ok(at)=>at,Err(error)=>{self.navigation_error=Some(error);0}};
        }
        if self.variable_edits.is_some() {
            self.rewind();
            let result=(||->Result<(i64,EditTime)> {
                let position=EditTime::signed_ticks(pts.max(0),self.track().timescale)?;
                let timeline=self.variable_edits.as_ref().unwrap();
                let position=if position.cmp(timeline.end)==std::cmp::Ordering::Greater {timeline.end}else{position};
                timeline.cursor.set(position);
                let position=timeline.locate().map_or(position,|i|position.max(timeline.minimum_starts[i]));
                timeline.cursor.set(position);
                let returned=i64::try_from(position.floor_ticks(self.track().timescale)?).map_err(|_|invalid("MP4 seek clock overflow"))?;
                Ok((returned,position))
            })();
            self.variable_edits.as_mut().unwrap().active=None;
            return match result {Ok((returned,_))=>returned,Err(error)=>{self.navigation_error=Some(error);0}};
        }
        if self.changing {
            let requested=pts.max(0).min(self.track().duration.min(i64::MAX as u64)as i64);
            let selected=self.track().samples.at_or_before(requested);
            let codec=self.track().description_for_sample(selected).unwrap().codec;
            if matches!(&codec,b"mp4a"|b"ac-3"|b"ec-3"|b"MAC3"|b"MAC6") {
                // A real setup change resets codec history during natural
                // playback, so earlier setups cannot contribute to this span.
                // Equivalent table entries still share that history.
                let setup=self.track().description_for_sample(selected).unwrap();
                let first=(0..selected).rev().find(|&i|self.track().description_for_sample(i)!=Some(setup)).map_or(0,|i|i+1);
                self.rewind();self.sample_index=first;self.presentation_floor=requested;return requested;
            }
            self.seek(requested);self.presentation_floor=requested;return requested;
        }
        if let Some(timeline)=&self.timeline {
            let rate=u128::from(self.sample_rate());let scale=u128::from(self.track().timescale);
            let cursor=(pts.max(0) as u128*rate/scale).min(u128::from(timeline.sample_frames)) as u64;
            self.edit_cursor.set(cursor);self.edit_segment.set(None);self.sample_index=0;
            return ((u128::from(cursor)*scale/rate).min(i64::MAX as u128)) as i64;
        }
        let track = self.track();
        let shift = if self.variable_edits.is_none() && track.codec == *b"mp4a" && track.edits.len() == 1 && track.edits[0].media_time >= 0 { track.edits[0].media_time } else { 0 };
        let preroll = matches!(&track.codec, b"ac-3" | b"ec-3" | b"MAC3" | b"MAC6")
            || (track.codec == *b"mp4a" && (track.edits.is_empty() || (track.edits.len() == 1 && track.edits[0].media_time >= 0)));
        let source_end=if track.duration>0 {track.duration as u128}else{
            track.samples.get(track.samples.len().saturating_sub(1)).map_or(0,|sample|sample.pts.max(0)as u128+u128::from(sample.duration))
        };
        let end=if preroll&&track.edits.len()==1&&self.demuxer.movie_timescale()!=0 {
            (shift.max(0)as u128+(u128::from(track.edits[0].duration)*u128::from(track.timescale)).div_ceil(u128::from(self.demuxer.movie_timescale()))).min(source_end)
        }else{source_end};
        let requested=pts.max(0).saturating_add(shift).max(0);
        let requested=if preroll {requested.min(end.min(i64::MAX as u128)as i64)}else{requested};
        let landed = Mp4AudioReader::seek(self, requested);
        if preroll {
            // Restore all decoder overlap/history before exposing the landed
            // packet. Preroll PCM is discarded by present_decoded.
            self.rewind();
            self.presentation_floor = requested;
            return requested;
        }
        self.presentation_floor = requested;
        landed
    }
}

#[cfg(test)]
mod tests {
    use super::Mp4AudioReader;
    use crate::audio::{AudioStream, AudioTrack};
    use crate::container::mp4::{Limits, Mp4Reader, SampleIndex};
    use std::io::Cursor;
    use std::mem::size_of;

    /// Two mono AAC tracks at different rates behind one H.264 track, written by
    /// ffmpeg so the listing comes from a real `trak` table rather than one
    /// assembled here:
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=2 \
    ///   -f lavfi -i sine=frequency=440:sample_rate=48000:duration=2 \
    ///   -f lavfi -i sine=frequency=880:sample_rate=32000:duration=2 \
    ///   -map 0:v -map 1:a -map 2:a -c:v libx264 -profile:v baseline \
    ///   -pix_fmt yuv420p -c:a aac -movflags +faststart two-audio.mp4
    /// ```
    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/two-audio.mp4");

    /// Every audio track a decoder exists for is listed in container order, and
    /// `open_at` takes a place in that list rather than a container index — the
    /// video track in between is skipped without the caller noticing.
    #[test]
    fn both_audio_tracks_are_listed_and_openable() {
        let first = Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("audio");
        assert_eq!(
            first.audio_tracks(),
            vec![
                AudioTrack {
                    sample_rate: 48_000,
                    channels: 1,
                    name: String::new(),
                    language: String::new(),
                },
                AudioTrack {
                    sample_rate: 32_000,
                    channels: 1,
                    name: String::new(),
                    language: String::new(),
                },
            ]
        );
        assert_eq!((first.sample_rate(), first.channels()), (48_000, 1));

        let second =
            Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).expect("second");
        assert_eq!((second.sample_rate(), second.channels()), (32_000, 1));
        assert_eq!(second.track().codec, *b"mp4a");
        // Past the end is an error, not a silent return to the first track.
        assert!(Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 2).is_err());
    }

    /// The second track is not just listed: its own setup record decodes, so a
    /// track switch does not hand the old track's configuration to the decoder.
    #[test]
    fn the_second_track_decodes_at_its_own_rate() {
        let mut stream =
            Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).expect("second");
        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("AAC decoder");
        let mut frames = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(
                    &packet.data,
                    packet.pts.max(0) as u64,
                    packet.duration.max(0) as u64,
                )
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 32_000));
            frames += pcm.data.len() / (size_of::<f32>() * usize::from(stream.channels()));
        }
        // Two seconds at 32 kHz, allowing for the encoder's priming and the
        // trailing frame the container still lists.
        assert!(
            (60_000..66_000).contains(&frames),
            "decoded {frames} frames"
        );
    }

    /// The whole PCM path over a real file: a QuickTime track the container used
    /// to refuse, indexed, decoded and timed. Half a second of a 0.75-amplitude
    /// 440 Hz sine at 48 kHz stereo, muxed beside an AVC picture:
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=0.5 \
    ///   -f lavfi -i 'aevalsrc=0.75*sin(880*PI*t)|0.75*sin(880*PI*t):d=0.5:s=48000' \
    ///   -map 0:v -map 1:a -c:v libx264 -pix_fmt yuv420p -preset ultrafast \
    ///   -c:a pcm_s16le tests/fixtures/audio/pcm-screen.mov
    /// ```
    ///
    /// ffmpeg writes the lossless `sowt` tag, and its `mov` muxer puts one audio
    /// frame in every sample, so the index lists 24000 of them against the 24
    /// blocks `ffprobe` shows when it coalesces them on read. The frames are all
    /// the same four bytes long, so they are indexed as runs of chunks rather than
    /// one record each, which is what lets a recording of any length in at all.
    #[test]
    fn a_pcm_track_decodes_sample_by_sample() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let mut stream =
            Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("PCM track");
        assert_eq!(stream.codec(), "sowt");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        // The sample entry's `sample_size`, which is what tells the decoder how
        // many bytes a sample has.
        assert_eq!(stream.bits_per_sample(), 16);
        assert!(stream.extra_data().is_empty(), "PCM has no setup record");

        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("PCM decoder");
        let bytes = usize::from(stream.channels()) * size_of::<f32>();
        let mut frames = 0usize;
        let mut packets = 0usize;
        let mut peak = 0.0f32;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let pcm = decoder
                .decode_encoded(
                    &packet.data,
                    packet.pts.max(0) as u64,
                    packet.duration.max(0) as u64,
                )
                .expect("decode")
                .expect("PCM always yields a packet");
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 48_000));
            // A block's bytes widen by exactly the ratio of the two widths.
            assert_eq!(pcm.data.len(), packet.data.len() * 2);
            frames += pcm.data.len() / bytes;
            packets += 1;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        assert_eq!((packets, frames), (24_000, 24_000));
        // 24576 of 32768, the exact top of the sine as ffmpeg's own decode of the
        // file reports it: lossless coding makes the equality exact.
        assert_eq!(peak, 0.75);
    }

    /// Every PCM tag QuickTime writes, in one file: the two integer byte orders, a
    /// float track, and a 24-bit big-endian track that keeps its width and order
    /// in child atoms (`wave`/`frma`/`enda`) this reader does not resolve.
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.75*sin(880*PI*t)|0.75*sin(880*PI*t):d=0.05:s=48000' \
    ///   -map 0:a -map 0:a -map 0:a -map 0:a -c:a:0 pcm_s16le -c:a:1 pcm_s16be \
    ///   -c:a:2 pcm_f32le -c:a:3 pcm_s24be tests/fixtures/audio/pcm-tags.mov
    /// ```
    ///
    /// Each of the four tags this player reads has to reach the decoder as its
    /// own layout, because a byte order or width guessed wrongly still yields
    /// 2400 frames of plausible-looking noise. The same 0.75-amplitude sine in all
    /// of them is what says the samples came out at the right scale and sign — and
    /// `fl32` is written as a version 1 entry, so it also checks that the fields
    /// read before the extra ones still line up.
    #[test]
    fn each_pcm_tag_is_read_at_its_own_width_and_order() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-tags.mov");
        let listing = Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("PCM");
        assert_eq!(listing.audio_tracks().len(), 4);
        assert!(Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 4).is_err());

        for (nth, codec, stated) in [
            (0, "sowt", 16u16),
            (1, "twos", 16),
            (2, "fl32", 16),
            (3, "in24", 24),
        ] {
            let mut stream = Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), nth)
                .unwrap_or_else(|error| panic!("track {nth}: {error}"));
            assert_eq!(stream.codec(), codec);
            assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
            // What the sample entry's `sample_size` states. `fl32` states 16 while
            // carrying 32-bit floats, which is why a float's width comes from the
            // fourcc and not from this field.
            assert_eq!(stream.bits_per_sample(), stated, "{codec} sample entry");
            let mut decoder = crate::codec::make_audio_decoder(
                stream.codec(),
                stream.extra_data(),
                stream.sample_rate(),
                stream.channels(),
                stream.bits_per_sample(),
            )
            .expect("PCM decoder");
            let mut frames = 0usize;
            let mut peak = 0.0f32;
            while let Some(packet) = stream.next_packet().expect("packet") {
                let pcm = decoder
                    .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                    .expect("decode")
                    .expect("PCM always yields a packet");
                frames += pcm.data.len() / (2 * size_of::<f32>());
                for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                    peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
                }
            }
            assert_eq!(frames, 2_400, "{codec}");
            assert_eq!(peak, 0.75, "{codec}");
        }
    }

    /// The total a listener with no picture is shown. Each track states its own
    /// duration in its own timescale, so the two tracks of the fixture count the
    /// same sound in 48000 and 32000 ticks; the half second of PCM counts in
    /// samples. AAC single-edit duration excludes encoder priming/padding and
    /// reports the two audible seconds in the presentation timeline.
    #[test]
    fn aac_duration_counts_leading_silence_and_repeated_ranges() {
        let data=include_bytes!("../../../tests/fixtures/playback-errors/aac-gap-repeat.m4a");
        let stream=Mp4AudioReader::open(Cursor::new(data),Limits::default()).unwrap();
        assert_eq!(stream.track().edits.len(),3);
        assert_eq!(stream.track().edits[0].media_time,-1);
        assert_eq!(stream.track().edits[1].media_time,stream.track().edits[2].media_time);
        assert_eq!(stream.duration(),Some(std::time::Duration::from_millis(220)));
        let mut pcm=Vec::new();
        let stats=crate::native_media::decode_mp4_aac_pcm(data,&mut pcm).unwrap();
        assert_eq!(stats.sample_frames,10560);
        assert_eq!(pcm.len(),10560*2*4);
        assert!(pcm[..960*2*4].iter().all(|b|*b==0));
        assert_eq!(&pcm[960*2*4..5760*2*4],&pcm[5760*2*4..]);
    }
    #[test]
    fn each_track_states_its_own_length() {
        const TWO: &[u8] = include_bytes!("../../../tests/fixtures/audio/two-audio.mp4");
        for (nth, total) in [(0, 2_000_000_000_u64), (1, 2_000_000_000)] {
            let stream = Mp4AudioReader::open_at(Cursor::new(TWO), Limits::default(), nth).unwrap();
            assert_eq!(
                stream.duration(),
                Some(std::time::Duration::from_nanos(total)),
                "track {nth}"
            );
        }
        const PCM: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let stream = Mp4AudioReader::open(Cursor::new(PCM), Limits::default()).unwrap();
        assert_eq!(
            stream.duration(),
            Some(std::time::Duration::from_millis(500))
        );
    }

    /// The frames of an uncompressed track all state the same width and the same
    /// length, so its index keeps one record per chunk instead of one per frame.
    /// The half second of the fixture is 24 000 frames over 13 chunks, which a
    /// budget of 100 records covers; the same limits used to drop the track and
    /// leave only the picture beside it. What bounded the length of PCM a file
    /// could carry was the index, not the sound.
    ///
    /// Reading the whole track through that budget is what says the compact
    /// records point at the same bytes the per-frame ones did: a stride wrong by
    /// two still yields four bytes of plausible sine wave.
    #[test]
    fn a_pcm_track_indexes_as_runs_of_chunks() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let tight = Limits {
            samples: 100,
            ..Limits::default()
        };
        let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), tight).expect("both tracks index");
        let audio = mp4
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .expect("PCM track");
        let SampleIndex::Uniform(runs) = &audio.samples else {
            panic!("expected runs of chunks, got {:?}", audio.samples);
        };
        assert_eq!(
            (
                runs.count,
                runs.runs.len(),
                runs.bytes_per_frame,
                runs.duration
            ),
            (24_000, 13, 4, 1)
        );
        // Inside a run the frames are the next four bytes and the next tick. The
        // runs themselves are not contiguous: the writer interleaves them with
        // the picture's chunks, which is the only reason there are 13 of them.
        for frame in 0..runs.count {
            let at = audio.samples.get(frame).unwrap();
            assert_eq!(
                (at.pts, at.dts, at.duration, at.size, at.sync),
                (frame as i64, frame as u64, 1, 4, true)
            );
        }
        let mut covered = 0usize;
        for run in &runs.runs {
            assert_eq!(run.first, covered, "runs follow one another");
            for frame in run.first..run.first + run.frames {
                let at = audio.samples.get(frame).unwrap();
                assert_eq!(at.offset, run.offset + 4 * (frame - run.first) as u64);
            }
            covered += run.frames;
        }
        assert_eq!(covered, runs.count);
        assert!(audio.samples.get(runs.count).is_none(), "no frame past end");

        let mut stream = Mp4AudioReader::open(Cursor::new(FIXTURE), tight).expect("PCM");
        let mut frames = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            assert_eq!((packet.pts, packet.duration), (frames as i64, 1));
            frames += 1;
        }
        assert_eq!(frames, 24_000);
        // A seek lands on the frame named, in a track it never indexed per frame.
        assert_eq!(stream.seek(12_345), 12_345);
        let packet = stream.next_packet().expect("packet").expect("frame");
        assert_eq!(packet.pts, 12_345);
    }

    /// The budget still means something: a track whose chunks alone outnumber it
    /// is left out the way an over-long one used to be, because the picture
    /// beside it is worth more than either. One record short of what the two
    /// tracks of the fixture together ask for is the smallest case that reaches
    /// the rule, and it only works because the picture is indexed first.
    #[test]
    fn a_pcm_track_over_the_sample_budget_leaves_the_picture() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let wide = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
        assert_eq!(wide.tracks()[0].handler, *b"vide", "picture indexed first");
        let cost = |handler: [u8; 4]| {
            wide.tracks()
                .iter()
                .filter(|t| t.handler == handler)
                .map(|t| t.samples.cost())
                .sum::<usize>()
        };
        let tight = Limits {
            samples: cost(*b"vide") + cost(*b"soun") - 1,
            ..Limits::default()
        };
        drop(wide);
        let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), tight).expect("the file still opens");
        assert_eq!(mp4.tracks().len(), 1);
        assert_eq!(mp4.tracks()[0].handler, *b"vide");
        assert!(Mp4AudioReader::open(Cursor::new(FIXTURE), tight).is_err());
    }

    /// The three audio tracks of the named file, listed the way the player
    /// shows them: the title where the file gives one, the language where it
    /// only states a language, and the layout the track carries after either.
    /// `tests/fixtures/tracks/named.mp4`, whose command `tests/mp4.rs` records.
    #[test]
    fn a_named_file_lists_its_audio_tracks_by_what_it_says() {
        const NAMED: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mp4");
        let stream = Mp4AudioReader::open(Cursor::new(NAMED), Limits::default()).expect("audio");
        let labels: Vec<String> = stream
            .audio_tracks()
            .iter()
            .map(crate::audio::AudioTrack::label)
            .collect();
        assert_eq!(
            labels,
            [
                "Первая · 1 ch 48000 Hz",
                "fre · 1 ch 32000 Hz",
                "1 ch 44100 Hz",
            ]
        );
    }
}

#[cfg(all(test, feature = "player"))]
mod edit_seek_tests {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn seek_anchor_applies_priming_offset_once() {
        let file = include_bytes!("../../../tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a").as_slice();
        let mut stream = Mp4AudioReader::open_at(std::io::Cursor::new(file),Default::default(),1).unwrap();
        let landed = stream.seek_to(2400);
        assert_eq!(landed,3408); // exact requested sample plus priming, not packet start
        assert_eq!(stream.time_of(landed),std::time::Duration::from_millis(50));
        let packet = stream.next_packet().unwrap().unwrap();
        assert_eq!(packet.pts,0); // seek primes the decoder from the beginning
        let zero=stream.seek_to(0);
        assert_eq!(zero,1008);
        assert_eq!(stream.time_of(zero),std::time::Duration::ZERO);
        assert_eq!(stream.time_of(0),std::time::Duration::ZERO);
    }
}

#[cfg(test)]
mod eac3_header_scan_tests {
    use super::*;
    use std::{cell::Cell,rc::Rc,io::{Cursor,SeekFrom}};
    struct Logged {data:Cursor<Vec<u8>>,bytes:Rc<Cell<usize>>}
    impl Read for Logged {
        fn read(&mut self,out:&mut[u8])->std::io::Result<usize> {
            let n=self.data.read(out)?;self.bytes.set(self.bytes.get()+n);Ok(n)
        }
    }
    impl Seek for Logged {
        fn seek(&mut self,at:SeekFrom)->std::io::Result<u64>{self.data.seek(at)}
    }
    #[test]
    fn selected_description_role_scan_reads_only_sixteen_byte_audio_headers() {
        let file=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-output-layout/top-late-plain.mp4");
        let bytes=Rc::new(Cell::new(0));
        let mut reader=Mp4Reader::open_indexed(Logged{data:Cursor::new(std::fs::read(file).unwrap()),bytes:bytes.clone()},Default::default()).unwrap();
        let track=reader.tracks().iter().position(|t|t.codec==*b"ec-3").unwrap();
        assert_eq!(reader.tracks()[track].samples.len(),162);
        bytes.set(0);
        let layout=Mp4AudioReader::<Logged>::description_speakers(&mut reader,track,0,0).unwrap().unwrap();
        assert_eq!(layout.0.len(),10);
        assert_eq!(bytes.get(),162*2*16);
    }
}
