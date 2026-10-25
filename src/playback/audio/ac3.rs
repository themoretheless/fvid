//! Audio playback of AC-3 out of the bare file the coding ships in.
//!
//! A `.ac3` file states nothing before its frames: each syncframe carries its own
//! header, and the geometry the player asks a container for - a codec tag, a rate,
//! a channel count, packets with timestamps - is read out of the frames
//! themselves, exactly as [`crate::playback_mp3`] does for MPEG audio. The tag
//! handed to [`crate::codec::make_audio_decoder`] is the one Matroska and MP4
//! already use for this coding, so the same decoder reads a track out of a
//! container and out of an elementary file.
//!
//! One measured fact shapes the walk beyond the header. A frame's length is not
//! stored in it as a count of bytes: the six-bit frame size code indexes
//! Section 5.18's table of 16-bit words, and the table's row depends on the
//! rate code, so the same code means a longer frame at 48 kHz than at 32 kHz.
//! Every frame of a stream decodes 1536 samples per channel whatever its rate,
//! which is what makes the timestamps a plain running count.
//!
//! The decoder is the work of [`crate::codec::ac3_decoder`], written from the
//! public ATSC A/52:2012 text; this file lists AC-3 and E-AC-3 frames through
//! their respective probes. They share the syncword but have different headers.
//! Multiplexed E-AC-3 exposes independent programmes as audio tracks. Only the
//! selected programme contributes sample time; other payloads are skipped by envelope.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::ac3_decoder::syncframe;
use crate::{Result, invalid};
use std::io::Read;
use std::cell::Cell;
use std::time::Duration;

/// What a limit guards: how much the reader may take in, and how many frames it
/// may agree to list.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most frames one stream may hold.
    pub packets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            packets: 1 << 20,
        }
    }
}

/// The tag [`crate::codec::make_audio_decoder`] answers for AC-3, as Matroska
/// names it; the MP4 fourcc `ac-3` reaches the same decoder.
pub const TAG: &str = "A_AC3";
pub const EAC3_TAG: &str = "A_EAC3";

/// Samples every syncframe decodes per channel, for all three rates: six blocks
/// of 256, which Section 5.4.1 fixes and Section 5.18's frame sizes follow.
const SAMPLES_PER_FRAME: u32 = 1536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SyncHeader {
    speaker_mask: u32,
    codec: &'static str,
    sample_rate: u32,
    channels: u16,
    frame_bytes: usize,
    samples: u32,
}

fn probe_syncframe(bytes: &[u8]) -> Option<SyncHeader> {probe_programme_syncframe(bytes,0)}
fn probe_programme_syncframe(bytes:&[u8],programme:u8)->Option<SyncHeader> {
    if programme==0 && let Some(at) = syncframe(bytes) {
        return Some(SyncHeader {
            speaker_mask: crate::codec::ac3_decoder::speaker_mask(bytes,at.channels).ok()?,
            codec: TAG,
            sample_rate: at.sample_rate,
            channels: at.channels,
            frame_bytes: at.frame_bytes,
            samples: SAMPLES_PER_FRAME,
        });
    }
    if let Some(at) = crate::codec::eac3_decoder::programme_frame(bytes,programme) {
        return Some(SyncHeader {
            speaker_mask: crate::codec::ac3_decoder::decoded_speaker_mask(usize::from(at.layout),at.low_frequency,at.channels).ok()?,
            codec: EAC3_TAG,
            sample_rate: at.sample_rate,
            channels: at.channels,
            frame_bytes: at.frame_bytes,
            samples: (at.blocks * 256) as u32,
        });
    }
    None
}

fn eac3_envelope(bytes:&[u8])->Option<(u8,u8,usize)> {
    let h=crate::codec::ac3_decoder::dolby_prefix(bytes)?;
    if h[..2]!=[0x0b,0x77] || !(11..=16).contains(&(h[5]>>3)){return None;}
    let kind=h[2]>>6;let id=(h[2]>>3)&7;
    let size=((usize::from(h[2])&7)*256+usize::from(h[3])+1)*2;
    (kind!=3 && size>=6 && size<=bytes.len()).then_some((kind,id,size))
}
fn first_sync_start(bytes:&[u8],opaque_eac3:bool)->(usize,Option<&'static str>){
    let mut at=0;
    while at+2<=bytes.len(){
        if let Some(h)=probe_syncframe(&bytes[at..]){return(at,Some(h.codec));}
        if opaque_eac3 && let Some((kind,0,_))=eac3_envelope(&bytes[at..]) && kind!=1{return(at,Some(EAC3_TAG));}
        at+=1;
    }
    (at,None)
}

// Best-effort catalogue: malformed/unrenderable unselected metadata must not
// invalidate a selected stream. Full validation belongs to the selected index.
fn programme_catalogue(mut bytes:&[u8],expanded:bool)->Vec<(u8,AudioTrack)> {
    let mut tracks:[Option<AudioTrack>;8]=std::array::from_fn(|_|None);
    while let Some((kind,id,size))=eac3_envelope(bytes) {
        if kind!=1 && tracks[usize::from(id)].is_none() && let Some(header)=probe_programme_syncframe(bytes,id) {
            let channels=if expanded {
                match crate::codec::eac3_decoder::presentation::raw_programme_for_programme(bytes,id) {
                    Ok(Some((_,layout))) if layout.0.len()>6=>Some(layout.0.len()as u16),
                    Ok(None)=>Some(header.channels),
                    Ok(Some((_,layout))) if layout.0.len()<=6=>Some(header.channels),
                    _=>None,
                }
            }else{Some(header.channels)};
            if let Some(channels)=channels {tracks[usize::from(id)]=Some(AudioTrack{sample_rate:header.sample_rate,channels,name:String::new(),language:String::new()});}
        }
        bytes=&bytes[size..];
    }
    tracks.into_iter().enumerate().filter_map(|(id,track)|track.map(|t|(id as u8,t))).collect()
}

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    pub pts: u64,
    /// The frame's own geometry, kept so a test can name what was read rather
    /// than a copy of the numbers the file states.
    pub rate: u32,
    pub channels: u16,
    pub samples: u32,
}

/// A `.ac3` or `.ec3` file and its indexed frames. AC-3 setup prefers a CRC-checked
/// header when one is available, retaining damaged frames for concealment.
#[derive(Clone, Debug)]
pub struct Ac3 {
    speaker_mask: Option<u32>,
    automatic_layout: Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>,
    programme:u8,
    tracks:Vec<(u8,AudioTrack)>,
    pub codec: &'static str,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: Vec<Frame>,
    data: Vec<u8>,
    truncated_final_frame: bool,
}

impl Ac3 {
    /// Read a whole file: the run of frames, from wherever the first syncframe
    /// starts to wherever the last one ends.
    pub fn parse(bytes:&[u8],limits:&Limits)->Result<Self>{Self::parse_mode(bytes,limits,true,0)}
    /// Compatibility output decodes only independent programme 0, up to 5.1.
    pub fn parse_compatible(bytes:&[u8],limits:&Limits)->Result<Self>{Self::parse_mode(bytes,limits,false,0)}
    /// Index only the chosen independent E-AC-3 programme and its extensions.
    pub fn parse_programme(bytes:&[u8],limits:&Limits,programme:u8)->Result<Self>{Self::parse_mode(bytes,limits,true,programme)}
    pub fn parse_compatible_programme(bytes:&[u8],limits:&Limits,programme:u8)->Result<Self>{Self::parse_mode(bytes,limits,false,programme)}
    pub fn programme(&self)->u8{self.programme}
    fn parse_mode(bytes: &[u8], limits: &Limits, expanded:bool,programme:u8) -> Result<Self> {
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        // The first header may sit anywhere in front of it, since a file whose head
        // was chopped off still names its frames from there on. After the first
        // header the run must be contiguous: every frame starts where the last one
        // ended, with no gap to scan across. That is what a real AC-3 stream looks
        // like end to end, and what a file of some other shape that happens to hold
        // two bytes resembling a syncword does not.
        let mut found: Vec<SyncHeader> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let (mut pos,first_codec)=first_sync_start(bytes,programme!=0);
        let first_start=pos;
        if programme!=0 && first_codec!=Some(EAC3_TAG){return Err(invalid("selected E-AC-3 programme is not present"));}
        let mut truncated_final_frame = false;
        let mut truncated_skipped_tail = false;
        loop {
            let remaining=bytes.get(pos..).unwrap_or_default();
            if first_codec==Some(EAC3_TAG) {
                match crate::codec::eac3_decoder::skipped_programme_bytes(remaining,programme) {
                    Ok(Some(size))=>{pos+=size;continue;},
                    Err(crate::Error::Invalid(_))=>{
                        truncated_final_frame=true;
                        truncated_skipped_tail=true;
                        break;
                    },
                    Err(reason)=>return Err(reason),
                    Ok(None)=>{},
                }
            }
            let Some(at)=probe_programme_syncframe(remaining,programme) else {break;};
            if let Some(first) = found.first()
                && at.codec != first.codec
            {
                break;
            }
            if pos + at.frame_bytes > bytes.len() {
                // A frame reaching past what the file holds is a truncated tail.
                truncated_final_frame = true;
                break;
            }
            if found.len() >= limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            found.push(at);
            starts.push(pos);
            pos += at.frame_bytes;
        }
        if found.is_empty() {
            if programme!=0{return Err(invalid("selected E-AC-3 programme is not present"));}
            return Err(invalid("no AC-3 syncframes in the file"));
        }
        // A run that stops well short of the end of the file was never the file: a
        // stream of frames fills it, and anything else is another container whose
        // bytes this reader happened to make a syncframe out of.
        // A recognized but incomplete unselected tail is still reported at EOF.
        // Its bytes must not make the complete primary frames look like a false
        // syncword match in some unrelated container.
        if !truncated_skipped_tail && (pos - first_start) * 10 < (bytes.len() - first_start) * 9 {
            return Err(invalid(&format!(
                "the frames run to byte {pos} of a file {} bytes long",
                bytes.len()
            )));
        }
        // A damaged AC-3 BSI must not establish the stream's channel/rate setup.
        // Keep its coded frame for silence concealment, but prefer a later
        // CRC-checked header when the raw file supplies no container metadata.
        let first_index=if found[0].codec==TAG {
            starts.iter().position(|&at|crate::codec::ac3_decoder::checked_syncframe(&bytes[at..]).is_some()).unwrap_or(0)
        }else{0};
        let first = found[first_index];
        let native=if expanded && first.codec==EAC3_TAG {
            crate::codec::eac3_decoder::presentation::raw_programme_for_programme(&bytes[starts[0]..],programme)?
                .map(|(_,layout)|layout).filter(|layout|layout.0.len()>6)
        }else{None};
        let output_channels=native.as_ref().map_or(first.channels,|layout|layout.0.len() as u16);
        let output_mask=if let Some(layout)=&native {
            layout.wave_mask()
        }else{Some(first.speaker_mask)};
        let mut role_union = std::collections::BTreeSet::new();
        let mut frames = Vec::with_capacity(found.len());
        let mut pts = 0u64;
        for (at, start) in found.iter().zip(&starts) {
            let checked=at.codec!=TAG || crate::codec::ac3_decoder::frame_crc_valid(&bytes[*start..*start+at.frame_bytes]);
            if checked && (at.sample_rate != first.sample_rate || at.channels != first.channels) {
                return Err(invalid(&format!(
                    "stream changes geometry at frame {}: {} Hz {} ch after {} Hz {} ch",
                    frames.len(),
                    at.sample_rate,
                    at.channels,
                    first.sample_rate,
                    first.channels
                )));
            }
            let (size,frame_channels)=if expanded && first.codec==EAC3_TAG {
                let programme=crate::codec::eac3_decoder::presentation::raw_programme_for_programme(&bytes[*start..],programme);
                match programme {
                    Ok(Some((size,layout))) => {
                        role_union.extend(layout.0.iter().copied());
                        (size,layout.0.len() as u16)
                    },
                    // Compatibility output still ignores incomplete skipped
                    // extensions. Complete bundles are retained so an explicit
                    // native output map can render replacement-only prefixes.
                    Ok(None) | Err(_) if native.is_none() => (at.frame_bytes,output_channels),
                    Err(_) if truncated_final_frame && Some(start)==starts.last()=>break,
                    Err(error)=>return Err(error),
                    _=>return Err(invalid("E-AC-3 elementary presentation changes or loses extension channels")),
                }
            }else{(at.frame_bytes,output_channels)};
            frames.push(Frame {
                start: *start,
                size,
                pts,
                rate: at.sample_rate,
                channels: frame_channels,
                samples: at.samples,
            });
            pts += u64::from(at.samples);
        }
        // Indexing already visits every selected programme. Negotiate a stable
        // canonical union without decoding twice or mixing unselected tracks.
        let automatic_layout = if !role_union.is_empty() && (1..=16).contains(&role_union.len()) {
            let layout = crate::codec::eac3_decoder::presentation::SpeakerLayout(role_union.into_iter().collect());
            layout.validate_output()?;
            Some(layout)
        } else { None };
        let output_channels = automatic_layout.as_ref().map_or(output_channels, |l| l.0.len() as u16);
        let output_mask = automatic_layout.as_ref().map_or(output_mask, |l| l.wave_mask());
        let own=AudioTrack{sample_rate:first.sample_rate,channels:output_channels,name:String::new(),language:String::new()};
        let mut tracks=if first.codec==EAC3_TAG{programme_catalogue(&bytes[first_start..],expanded)}else{Vec::new()};
        if let Some((_,track))=tracks.iter_mut().find(|(id,_)|*id==programme){*track=own;}else{tracks.push((programme,own));}
        tracks.sort_by_key(|(id,_)|*id);
        if tracks.len()>1 {for(id,track)in &mut tracks{track.name=format!("E-AC-3 programme {id}");}}
        Ok(Self {
            programme,tracks,
            speaker_mask: output_mask,
            automatic_layout,
            codec: first.codec,
            sample_rate: first.sample_rate,
            channels: output_channels,
            frames,
            data: bytes.to_vec(),
            truncated_final_frame,
        })
    }

    /// Frames the stream hands over as packets.
    pub fn packets(&self) -> usize {
        self.frames.len()
    }

    /// One frame's bytes, header included, as the decoder expects them.
    pub fn packet(&self, index: usize) -> &[u8] {
        let at = self.frames[index.min(self.frames.len() - 1)];
        &self.data[at.start..at.start + at.size]
    }

    /// Samples the stream runs to, which is its length for a file that states no
    /// duration of its own.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(at.samples))
            .unwrap_or(0)
    }
}

/// A `.ac3` or `.ec3` file read as an audio track.
pub struct Ac3AudioReader {
    ac3: Ac3,
    eac3_stereo_output: bool,
    eac3_output_layout: Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>,
    ac3_stereo_output: bool,
    packet: usize,
    presentation_trim: Cell<u64>,
    presentation_floor: Option<u64>,
}

impl Ac3AudioReader {
    /// Read a whole file. The frames lie at lengths only their own headers state,
    /// so all of them are walked before the first one plays.
    pub fn open<R:Read>(reader:R,limits:Limits)->Result<Self>{Self::open_mode(reader,limits,true,0)}
    pub fn open_compatible<R:Read>(reader:R,limits:Limits)->Result<Self>{Self::open_mode(reader,limits,false,0)}
    /// Open encoded independent programme ID 0..7, retaining native extensions.
    pub fn open_programme<R:Read>(reader:R,limits:Limits,programme:u8)->Result<Self>{
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        Self::open_mode(reader,limits,true,programme)
    }
    /// Open one encoded programme with independent-only output up to 5.1.
    pub fn open_compatible_programme<R:Read>(reader:R,limits:Limits,programme:u8)->Result<Self>{
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        Self::open_mode(reader,limits,false,programme)
    }
    /// Zero-based track positions follow the catalogue of decodable programmes.
    /// Unlike encoded IDs, positions omit unrenderable programme metadata.
    pub fn open_track<R:Read>(reader:R,limits:Limits,nth:usize)->Result<Self>{
        if nth>=8{return Err(invalid("AC-3 audio track index is out of range"));}
        if nth==0{return Self::open(reader,limits);}
        let bytes=Self::read_limited(reader,limits)?;
        let(start,codec)=first_sync_start(&bytes,true);
        if codec!=Some(EAC3_TAG){return Err(invalid("AC-3 audio track index is out of range"));}
        let tracks=programme_catalogue(&bytes[start..],true);
        let programme=tracks.get(nth).ok_or_else(||invalid("AC-3 audio track index is out of range"))?.0;
        Self::from_bytes(&bytes,limits,true,programme)
    }
    fn read_limited<R:Read>(mut reader:R,limits:Limits)->Result<Vec<u8>> {
        let mut bytes=Vec::new();
        let cap=u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len()>limits.file_bytes{return Err(invalid(&format!("file is over the {} byte limit",limits.file_bytes)));}
        Ok(bytes)
    }
    fn from_bytes(bytes:&[u8],limits:Limits,expanded:bool,programme:u8)->Result<Self>{
        Ok(Self{ac3:Ac3::parse_mode(bytes,&limits,expanded,programme)?,eac3_stereo_output:false,eac3_output_layout:None,ac3_stereo_output:false,packet:0,presentation_trim:Cell::new(0),presentation_floor:None})
    }
    fn open_mode<R:Read>(reader:R,limits:Limits,expanded:bool,programme:u8)->Result<Self>{
        let bytes=Self::read_limited(reader,limits)?;
        Self::from_bytes(&bytes,limits,expanded,programme)
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn ac3(&self) -> &Ac3 {
        &self.ac3
    }

    fn audible_samples(&self) -> u64 {
        self.ac3.samples().saturating_sub(crate::codec::ac3_decoder::ENCODER_DELAY as u64)
    }
}

impl AudioStream for Ac3AudioReader {
    fn set_eac3_output_layout(&mut self, layout:Option<crate::codec::eac3_decoder::presentation::SpeakerLayout>)->Result<()> {
        if let Some(l)=&layout {if self.codec()!=EAC3_TAG {return Err(invalid("native E-AC-3 layout requires an E-AC-3 source"));}l.validate_output()?;}
        self.eac3_output_layout=layout;Ok(())
    }
    fn set_ac3_stereo_output(&mut self, enabled:bool)->Result<()> {self.ac3_stereo_output=enabled;Ok(())}
    fn stereo_router(&self)->Result<Option<fvid_media::owned_stereo_router::StereoRouter>> {
        let mask=self.speaker_mask()?;
        if mask.is_none() && self.codec()==EAC3_TAG && self.channels()>6 {return Err(crate::unsupported("E-AC-3 speaker locations need an extended speaker mask"));}
        mask.map(|m|fvid_media::owned_stereo_router::StereoRouter::new(self.channels(),m).map_err(|e|invalid(&e))).transpose()
    }
    fn set_eac3_stereo_output(&mut self, enabled: bool) -> Result<()> {
        self.eac3_stereo_output=enabled;
        Ok(())
    }
    fn make_decoder(&self)->Result<Box<dyn crate::audio::AudioDecode>> {
        if self.ac3_stereo_output && self.codec()==TAG {return Ok(Box::new(crate::codec::ac3_decoder::Ac3Decoder::new_stereo(self.extra_data(),self.sample_rate())?));}
        if self.codec()==EAC3_TAG {
            if self.eac3_stereo_output {
                return Ok(Box::new(crate::codec::eac3_decoder::Eac3Decoder::new_stereo(self.extra_data(),self.sample_rate())?.with_programme(self.ac3.programme)?));
            }
            {
                let mut d=Box::new(crate::codec::eac3_decoder::Eac3Decoder::new(self.extra_data(),self.sample_rate(),self.channels())?.with_programme(self.ac3.programme)?);
                if let Some(layout)=self.eac3_output_layout.as_ref().or(self.ac3.automatic_layout.as_ref()) {d.set_output_layout(layout.clone())?;}
                Ok(d)
            }
        }else{crate::codec::make_audio_decoder(self.codec(),self.extra_data(),self.sample_rate(),self.channels(),self.bits_per_sample())}
    }
    fn audio_track_index(&self)->usize {self.ac3.tracks.iter().position(|(id,_)|*id==self.ac3.programme).unwrap_or(0)}

    fn speaker_mask(&self) -> Result<Option<u32>> {Ok(if (self.eac3_stereo_output && self.codec()==EAC3_TAG)||(self.ac3_stereo_output && self.codec()==TAG) {Some(3)}else if self.codec()==EAC3_TAG && let Some(l)=&self.eac3_output_layout {l.wave_mask()}else{self.ac3.speaker_mask})}

    fn codec(&self) -> &str {
        self.ac3.codec
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.ac3.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.ac3.sample_rate
    }

    /// The frame's own native layout, which is the one the decoder answers with
    /// when a container states none: five full-bandwidth channels plus an LFE for
    /// a 5.1 stream, two for stereo, one for mono.
    fn channels(&self) -> u16 {
        if (self.eac3_stereo_output && self.codec()==EAC3_TAG)||(self.ac3_stereo_output && self.codec()==TAG) {2}else if self.codec()==EAC3_TAG && let Some(l)=&self.eac3_output_layout {l.0.len()as u16}else{self.ac3.channels}
    }

    fn duration(&self) -> Option<Duration> {
        let samples = self.audible_samples();
        let rate = u64::from(self.ac3.sample_rate);
        Some(Duration::new(samples / rate, ((samples % rate) * 1_000_000_000 / rate) as u32))
    }

    fn present_decoded(&self, mut packet: crate::audio::AudioPacket, source_pts: i64) -> Result<Option<crate::audio::AudioPacket>> {
        let stride = usize::from(self.channels()) * 4;
        if stride == 0 || !packet.data.len().is_multiple_of(stride) {
            return Err(invalid("invalid raw AC-3 decoded geometry"));
        }
        let frames = packet.data.len() / stride;
        let window = self
            .ac3
            .frames
            .iter()
            .find(|at| at.pts as i64 == source_pts)
            .map(|at| at.samples as usize)
            .unwrap_or(SAMPLES_PER_FRAME as usize);
        if frames > window {
            return Err(invalid("raw AC-3 packet exceeds its frame window"));
        }
        let trim = self.presentation_trim.get();
        packet.pts = u64::try_from(source_pts).ok().and_then(|pts| pts.checked_sub(trim))
            .ok_or_else(|| invalid("raw AC-3 presentation precedes its source timeline"))?;
        packet.timebase_num = 1;
        packet.timebase_den = self.sample_rate();
        self.presentation_trim.set(trim + (window - frames) as u64);
        let skip = self.presentation_floor.unwrap_or(0).saturating_sub(packet.pts).min(frames as u64) as usize;
        if skip == frames { return Ok(None); }
        if skip != 0 {
            packet.data.copy_within(skip * stride.., 0);
            packet.data.truncate((frames - skip) * stride);
            packet.pts += skip as u64;
        }
        Ok(Some(packet))
    }

    fn validate_eof(&self) -> Result<()> {
        if self.ac3.truncated_final_frame {
            return Err(invalid("AC-3 final frame is truncated"));
        }
        Ok(())
    }

    /// A frame describes itself: this container stores no setup block.
    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {self.ac3.tracks.iter().map(|(_,track)|track.clone()).collect()}

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.ac3.frames.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.ac3.packet(self.packet).to_vec(),
            pts: at.pts as i64,
            duration: i64::from(at.samples),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
        self.presentation_trim.set(0);
        self.presentation_floor = None;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        self.presentation_trim.set(0);
        let target = (pts.max(0) as u64).min(self.audible_samples());
        self.presentation_floor = Some(target);
        if target >= self.audible_samples() {
            self.packet = self.ac3.frames.len();
            return self.audible_samples() as i64;
        }
        // Replay history for overlap/dither fidelity, then hide PCM before target.
        self.packet = 0;
        target as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Ac3, Ac3AudioReader, Limits, SAMPLES_PER_FRAME, TAG};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::{
        ac3_decoder::{Syncframe, syncframe},
        make_audio_decoder,
    };

    /// The four files under `tests/fixtures/audio`, each an AC-3 elementary stream
    /// written by this machine's FFmpeg and covering the three rates and the channel
    /// layouts the coding reaches: 48 kHz stereo at 192 and 256 kbps (the second is
    /// the coupled frame the decoder's Section 7.3.4 work is measured on), 48 kHz
    /// 5.1 at 448 kbps, and mono at 32 kHz and 96 kbps. The frame lists asserted
    /// below are ffprobe's, packet for packet, and its reported bit rates and
    /// durations match the counts and lengths here: eight frames of 768, 1024 and
    /// 1792 bytes and six of 576, each file ending where its frames do. The signal
    /// inside them is a tone, but the encoder's input command was not written down
    /// when these files were cut, so nothing here claims a byte-for-byte
    /// reproduction of them - only the geometry they state is asserted.
    const STEREO: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-stereo.ac3");
    const COUPLED: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-coupled.ac3");
    const SURROUND: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-51.ac3");
    const MONO_32K: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-mono-32k.ac3");

    fn parse(bytes: &[u8]) -> Ac3 {
        Ac3::parse(bytes, &Limits::default()).expect("parses")
    }

    /// The frame walk is a run of lengths the headers state, so the whole-file
    /// assertions are about coverage: the frames must end where the file does.
    #[test]
    fn a_real_file_is_exactly_the_frames_its_header_counts() {
        for (bytes, rate, channels, count) in [
            (STEREO, 48_000, 2, 8),
            (COUPLED, 48_000, 2, 8),
            (SURROUND, 48_000, 6, 8),
            (MONO_32K, 32_000, 1, 6),
        ] {
            let ac3 = parse(bytes);
            assert_eq!(ac3.sample_rate, rate);
            assert_eq!(ac3.channels, channels);
            assert_eq!(ac3.packets(), count, "frames at {rate} Hz {channels} ch");
            assert_eq!(
                ac3.frames.last().map(|at| at.start + at.size),
                Some(bytes.len()),
                "the stream ends where the file does"
            );
            assert!(
                ac3.frames
                    .iter()
                    .all(|at| at.rate == rate && at.channels == channels)
            );
        }
    }

    /// Section 5.18 puts the same frame size code at different byte lengths for
    /// the three rates, and the walk trusts the table it is read out of.
    #[test]
    fn a_frames_length_comes_from_its_rate_and_size_code() {
        let stereo = parse(STEREO);
        assert_eq!(stereo.frames[0].size, 768, "192 kbps at 48000 Hz");
        let surround = parse(SURROUND);
        assert_eq!(surround.frames[0].size, 1792, "448 kbps at 48000 Hz");
        let mono = parse(MONO_32K);
        assert_eq!(mono.frames[0].size, 576, "96 kbps at 32000 Hz");
        // Every frame of the file states the same length, which is what makes the
        // walk a stride rather than a search.
        assert!(stereo.frames.iter().all(|at| at.size == 768));
    }

    #[test]
    fn timestamps_count_the_samples_a_frame_always_holds() {
        let ac3 = parse(SURROUND);
        for (index, at) in ac3.frames.iter().enumerate() {
            assert_eq!(at.pts, index as u64 * u64::from(SAMPLES_PER_FRAME));
        }
        assert_eq!(ac3.samples(), 8 * 1536);
        let reader = Ac3AudioReader::open(SURROUND, Limits::default()).expect("opens");
        assert_eq!(reader.timescale(), 48_000);
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_nanos(250_666_666)),
            "eight frames less the native256sample leading trim"
        );
    }

    /// The reader is only useful if the decoder accepts what it hands over, and
    /// the tag is the one the dispatch answers for.
    #[test]
    fn the_packets_it_hands_over_decode_as_the_tag_says() {
        let mut reader = Ac3AudioReader::open(STEREO, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), TAG);
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 2));
        assert_eq!(
            reader.bits_per_sample(),
            0,
            "a compressed stream says no width"
        );
        assert!(reader.extra_data().is_empty());
        assert_eq!(reader.audio_tracks()[0].label(), "2 ch 48000 Hz");
        let mut decoder =
            make_audio_decoder(TAG, &[], 48_000, 2, 0).expect("the dispatch answers this tag");
        let mut heard = 0usize;
        let mut stamps = Vec::new();
        while let Some(EncodedPacket {
            data,
            pts,
            duration,
        }) = reader.next_packet().expect("packet")
        {
            stamps.push((pts, duration));
            let audio = decoder
                .decode_encoded(&data, pts as u64, duration as u64)
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            // The first block carries the encoder's 256 samples of padding, which
            // the decoder withholds so the sound starts where every other player
            // starts it; the frames after it sound their full length.
            let frames = 1536 - 256 * usize::from(stamps.len() == 1);
            assert_eq!(
                audio.data.len() / 4,
                frames * 2,
                "one frame of interleaved f32 per channel"
            );
            assert_eq!((audio.pts, audio.timebase_den), (pts as u64, 48_000));
            heard += audio.data.len() / 4;
        }
        assert_eq!(stamps.len(), reader.ac3().packets());
        assert_eq!(stamps[1], (1536, 1536), "pts is samples, not bytes");
        assert_eq!(heard, 8 * 1536 * 2 - 256 * 2, "every frame sounded its samples");
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn a_seek_restores_history_before_the_requested_audible_sample() {
        let mut reader = Ac3AudioReader::open(SURROUND, Limits::default()).expect("opens");
        assert_eq!(reader.seek_to(4000), 4000, "seek is sample accurate");
        assert_eq!(reader.next_packet().unwrap().unwrap().pts,0,"encoded history is preroll");
        assert_eq!(reader.seek_to(3072), 3072, "exact requested sample");
        assert_eq!(reader.seek_to(0), 0);
        assert_eq!(
            reader.seek_to(1 << 30),
            12_032,
            "past the audible end is exhausted"
        );
        assert!(reader.next_packet().expect("packet").is_none());
        reader.rewind();
        assert_eq!(reader.seek_to(-5), 0, "a negative sample is the start");
    }

    /// A file of another shape must be refused rather than half-read: the player
    /// offers every reader in turn, so a false accept costs the file its real
    /// codec. Both guards below were checked against the bytes of files this
    /// machine holds.
    #[test]
    fn a_file_that_is_not_a_run_of_syncframes_is_refused() {
        let mp3 = include_bytes!("../../../tests/fixtures/mp3/tone.mp3");
        let error = Ac3::parse(mp3, &Limits::default()).expect_err("an MP3 is not AC-3");
        assert!(error.to_string().contains("no AC-3 syncframes"), "{error}");
        // Bytes that name a syncword but run out mid-frame are a stream that stops
        // short of the file, and one frame of a real run is not a false accept: a
        // truncated tail is the file's own damage, not another container's.
        let mut chopped = STEREO.to_vec();
        chopped.truncate(3 * 768 + 100);
        assert_eq!(parse(&chopped).packets(), 3, "only the frames that fit");
        // A tail that reaches into a file of another shape is the case the coverage
        // guard is for: fill the rest with bytes no header survives.
        let mut padded = STEREO[..768].to_vec();
        padded.extend(vec![0u8; 4000]);
        let error = Ac3::parse(&padded, &Limits::default()).expect_err("padding is not audio");
        assert!(error.to_string().contains("run to byte"), "{error}");
        // The header probe reads the geometry out of the frame's own bytes, so the
        // walk never guesses a length it cannot find in the table.
        assert_eq!(
            syncframe(&STEREO[768..]),
            Some(Syncframe {
                sample_rate: 48_000,
                channels: 2,
                frame_bytes: 768,
            })
        );
        assert_eq!(syncframe(&[0x0b, 0x77, 0, 0]), None, "a stub is no header");
    }
}
