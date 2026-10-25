//! WebM audio packet extraction.
//!
//! Extracts encoded audio packets from WebM containers for decoding.

use crate::audio::{AudioStream, EncodedPacket};
use crate::container::webm::{Limits, Track, WebmReader};
use crate::{Result, invalid, unsupported};
use std::io::{Read, Seek};

/// Timestamps in a Matroska segment are nanoseconds.
const TIMESCALE_NS: u32 = 1_000_000_000;

/// Codec IDs the player has a decoder for, as Matroska spells them. The three
/// `A_PCM/*` IDs are uncompressed audio, whose width the track states in
/// `BitDepth`. `A_AAC` is spelled `mp4a` by the decoder dispatch, whichever
/// container named it, and needs its setup block rebuilt (see `aac_setup`).
const CODECS: [&str; 16] = [
    "A_OPUS",
    "A_VORBIS",
    "A_MPEG/L3",
    "A_MPEG/L2",
    "A_MPEG/L1",
    "A_FLAC",
    "A_ALAC",
    "A_WAVPACK4",
    "A_AC3",
    "A_AC3/BSID9",
    "A_AC3/BSID10",
    "A_EAC3",
    "A_AAC",
    "A_PCM/INT/LIT",
    "A_PCM/INT/BIG",
    "A_PCM/FLOAT/IEEE",
];

/// The codec setup bytes a decoder needs for a track. Vorbis keeps its header
/// packets in `CodecPrivate`; FLAC keeps its metadata blocks, from which the
/// decoder reads only the STREAMINFO body; Apple Lossless keeps its magic cookie,
/// which is the whole of its geometry; both MPEG audio layers describe their own
/// frames and want nothing.
fn setup_data(track: &Track) -> Result<Vec<u8>> {
    match track.codec.as_str() {
        "A_OPUS" => {
            let channels = crate::codec::opus_decoder::header(&track.codec_private)?.channel_count;
            if track.channels != u64::from(channels) {
                return Err(invalid("Opus header disagrees with Matroska channels"));
            }
            Ok(track.codec_private.clone())
        }
        "A_VORBIS" => vorbis_setup_headers(&track.codec_private),
        "A_FLAC" => Ok(flac_stream_info(&track.codec_private)),
        "A_ALAC" | "A_WAVPACK4" => Ok(track.codec_private.clone()),
        "A_AAC" => aac_setup(&track.codec_private),
        _ => Ok(Vec::new()),
    }
}

/// Matroska states an AAC track's setup as the bare AudioSpecificConfig, while
/// the decoder here reads an MP4 `esds` box that holds the same bytes inside its
/// descriptors. A file written by a tool that knows only Matroska carries the
/// short form, so the wrapper is rebuilt around it; a track whose setup block is
/// too short to name a coding is refused by the reason the config parser gives.
fn aac_setup(private: &[u8]) -> Result<Vec<u8>> {
    let Some(esds) = crate::playback_aac::esds_for(private) else {
        return Err(unsupported(&format!(
            "AAC in WebM names its setup block {} bytes, and a config shorter than two states no coding",
            private.len()
        )));
    };
    // Let the decoder's own parser judge the rebuilt box, so a config this
    // reader wraps wrongly is refused here rather than two frames in.
    crate::codec::config::aac_specific_config(&esds)?;
    Ok(esds)
}

/// The STREAMINFO body of a FLAC setup block. Matroska stores the metadata
/// blocks in `CodecPrivate`, sometimes with the `fLaC` marker some writers keep
/// in front; a block is a flag byte, a 24-bit length and its body, and the
/// decoder reads only the 34-byte STREAMINFO body, so its header goes too.
fn flac_stream_info(private: &[u8]) -> Vec<u8> {
    let blocks = private.strip_prefix(b"fLaC").unwrap_or(private);
    if blocks.len() >= 38 && blocks[0] & 0x7f == 0 && blocks[1..4] == [0, 0, 34] {
        return blocks[4..38].to_vec();
    }
    blocks.to_vec()
}

/// Codec padding sample counts are quantized into integer nanoseconds.
/// Restore the nearest count, matching owned decode; seek boundaries still ceil.
fn discard_padding_samples(padding_ns: i64, rate: u32) -> u128 {
    (u128::from(padding_ns.unsigned_abs()) * u128::from(rate) + 500_000_000) / 1_000_000_000
}

// Vorbis padding describes a time interval: even a fractional sample
// must be removed so no presented sample extends beyond the endpoint.
fn vorbis_padding_samples(padding_ns: i64, rate: u32) -> u128 {
    let ns = u128::from(padding_ns.unsigned_abs());
    let scaled = ns * u128::from(rate);
    let nearest = (scaled + 500_000_000) / 1_000_000_000;
    // A nonzero whole-sample count can be rounded to either adjacent ns
    // by the muxer. Preserve it, but never erase a positive sub-sample trim.
    if nearest > 0 && scaled.abs_diff(nearest * 1_000_000_000) <= u128::from(rate) {
        nearest
    } else {
        scaled.div_ceil(1_000_000_000)
    }
}

fn discard_padding_exceeds_packet(padding_ns: i64, rate: u32, frames: usize) -> bool {
    if rate == 0 { return true; }
    let duration_ns = (frames as u128 * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate);
    u128::from(padding_ns.unsigned_abs()) > duration_ns
}

/// Independent integer-ns clock rounding can put an exact sample boundary
/// one ns below zero. Preserve that sample; genuine off-grid preroll still ceil.
fn zero_origin_preroll_samples(source_pts: i64, rate: u32, frames: usize) -> usize {
    if rate == 0 { return frames; }
    let scaled = (-i128::from(source_pts)).max(0) as u128 * u128::from(rate);
    let nearest = (scaled + 500_000_000) / 1_000_000_000;
    let at = i128::from(source_pts) + (nearest * 1_000_000_000 / u128::from(rate)) as i128;
    let skip = if at >= -1 { nearest } else { scaled.div_ceil(1_000_000_000) };
    skip.min(frames as u128) as usize
}

fn presentation_after_preroll(source_pts: i64, first: usize, rate: u32) -> Option<u64> {
    if rate == 0 { return None; }
    let at = i128::from(source_pts) + first as i128 * 1_000_000_000 / i128::from(rate);
    u64::try_from(if at == -1 { 0 } else { at }).ok()
}

/// Recover a sample point hidden by nanosecond quantization, without rounding
/// an arbitrary requested boundary backwards. Coarse/off-grid packet clocks
/// retain their declared origin and strict ceil behavior.
fn aac_sample_window(
    source_pts: i64,
    floor: i64,
    rate: u32,
    frames: usize,
    origin_ns: i64,
    delay_ns: u64,
) -> (usize, i128) {
    let rate = i128::from(rate);
    let unit = 1_000_000_000i128;
    let delay = ((u128::from(delay_ns) * rate as u128 + 500_000_000) / 1_000_000_000) as i128;
    let origin = i128::from(origin_ns) * rate - delay * unit;
    let source = i128::from(source_pts) * rate - origin;
    let sample = (source + unit / 2).div_euclid(unit);
    if (source - sample * unit).abs() <= 2 * rate {
        let requested = i128::from(floor) * rate - origin;
        let target = requested.div_euclid(unit) + i128::from(requested.rem_euclid(unit) != 0);
        let skip = (target - sample).clamp(0, frames as i128) as usize;
        (
            skip,
            (origin + (sample + skip as i128) * unit).div_euclid(rate),
        )
    } else {
        let delta = (i128::from(floor) - i128::from(source_pts)).max(0) * rate;
        let skip = (delta.div_euclid(unit) + i128::from(delta.rem_euclid(unit) != 0))
            .min(frames as i128) as usize;
        (skip, i128::from(source_pts) + skip as i128 * unit / rate)
    }
}
/// Audio packet with presentation timestamp.
pub struct AudioPacket {
    /// Encoded audio data (Vorbis frame).
    pub data: Vec<u8>,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Track number this packet belongs to.
    pub track: u64,
}

struct VorbisClock {
    blocks: [u8; 2],
    modes: Vec<fvid_codecs::vorbis::Mode>,
    previous: Option<i64>,
    origin: Option<i64>,
    samples: u64,
    end_ns: Option<u64>,
}
impl VorbisClock {
    fn reset(&mut self) {
        self.previous = None;
        self.origin = None;
        self.samples = 0;
        self.end_ns = None;
    }
    fn stamp(&mut self, data: &[u8], mut pts: i64, rate: u32, tick: u64) -> Result<(i64, i64)> {
        let header = fvid_codecs::vorbis::PacketMode::parse(
            &mut fvid_codecs::vorbis::PacketBits::new(data),
            &self.modes,
        )?;
        let n = 1i64 << self.blocks[usize::from(self.modes[header.mode].long)];
        let frames = self.previous.map_or(0, |previous| (previous + n) / 4) as u64;
        self.previous = Some(n);
        if let Some(origin) = self.origin {
            let expected =
                i128::from(origin) + i128::from(self.samples) * 1_000_000_000 / i128::from(rate);
            let expected = i64::try_from(expected).map_err(|_| invalid("Vorbis clock overflow"))?;
            if pts.abs_diff(expected) <= tick {
                pts = expected;
            } else {
                self.origin = Some(pts);
                self.samples = 0;
            }
        } else {
            self.origin = Some(pts);
        }
        self.samples = self
            .samples
            .checked_add(frames)
            .ok_or_else(|| invalid("Vorbis sample clock overflow"))?;
        let end = i128::from(self.origin.unwrap())
            + i128::from(self.samples) * 1_000_000_000 / i128::from(rate);
        let duration = i64::try_from(end - i128::from(pts))
            .map_err(|_| invalid("Vorbis duration overflow"))?;
        self.end_ns = Some(u64::try_from(end.max(0)).map_err(|_| invalid("Vorbis end overflow"))?);
        Ok((pts, duration))
    }
}

#[derive(Default)]
struct OpusClock {
    timeline: MpegClock,
    delay_ns:u64,
    next: Option<i64>,
    samples: u64,
    packets: usize,
}

struct OpusTiming {
    timeline:MpegClock,
    delay_ns:u64,
    pts: i64,
    frames: usize,
    preskip: usize,
}

impl OpusClock {
    fn stamp(&mut self, mut pts: i64, duration: u64, preskip: u64, tick: u64, grid:Option<fvid_media::owned_matroska_grid_clock::PacketGrid>, factor:f64, delay_ns:u64) -> Result<OpusTiming> {
        let frames = duration.checked_mul(48000)
            .ok_or_else(|| invalid("Opus packet duration overflow"))? / 1_000_000_000;
        let skip = preskip.saturating_sub(self.samples).min(frames);
        let samples = self.samples.checked_add(frames)
            .ok_or_else(|| invalid("Opus sample counter overflow"))?;
        let packets = self.packets.checked_add(1)
            .ok_or_else(|| invalid("Opus packet counter overflow"))?;
        if self.next.is_none() && pts >= 0 {
            if fvid_media::owned_matroska_sample_clock::quantized_sample_matches(pts,0,0,48000,tick) { pts = 0; }
            let priming = i64::try_from(u128::from(preskip) * 1_000_000_000 / 48000)
                .map_err(|_| invalid("Opus priming clock overflow"))?;
            pts = pts.checked_sub(priming).ok_or_else(|| invalid("Opus priming clock overflow"))?;
        }
        let count=usize::try_from(frames).map_err(|_|invalid("Opus frame count overflow"))?;
        pts=self.timeline.stamp_aac(pts,count,48000,tick,grid,factor)?.0;
        let duration = i64::try_from(duration).map_err(|_| invalid("Opus packet clock overflow"))?;
        let next = pts.checked_add(duration).ok_or_else(|| invalid("Opus packet clock overflow"))?;
        let frames = usize::try_from(frames).map_err(|_| invalid("Opus frame count overflow"))?;
        self.next = Some(next); self.samples = samples; self.packets = packets;
        self.delay_ns=delay_ns;
        Ok(OpusTiming { timeline:self.timeline.clone(),delay_ns,pts, frames, preskip: skip as usize })
    }
}

impl OpusTiming {
    fn audible_end(&self, padding_ns: i64) -> Result<Option<u64>> {
        let padding = discard_padding_samples(padding_ns, 48000);
        if padding > self.frames as u128 || discard_padding_exceeds_packet(padding_ns, 48000, self.frames) {
            return Err(invalid("Opus discard padding exceeds decoded packet"));
        }
        let first = self.preskip.max(self.timeline.exact_skip(0,48000,self.frames,self.delay_ns).ok_or_else(||invalid("invalid Opus sample clock"))?)
            .max(if padding_ns < 0 { padding as usize } else { 0 });
        let last = self.frames - if padding_ns > 0 { padding as usize } else { 0 };
        if first >= last { return Ok(None); }
        let end=self.timeline.exact_point(last,48000,self.delay_ns).ok_or_else(||invalid("negative Opus presentation end"))?;
        Ok(Some(end))
    }
}

/// Exact MPEG sample coordinates beneath quantized Matroska block timestamps.
#[derive(Clone, Default)]
struct MpegClock {
    origin: Option<i64>,
    samples: u64,
    packet_start: u64,
    grid_origin: Option<fvid_media::owned_matroska_grid_clock::PacketGrid>,
}
impl MpegClock {
    fn stamp(&mut self, source: i64, frames: usize, rate: u32, tick: u64) -> Result<(i64,i64)> {
        self.stamp_impl(source,frames,rate,tick,None)
    }
    fn stamp_aac(&mut self, source: i64, frames: usize, rate: u32, tick: u64,
        grid:Option<fvid_media::owned_matroska_grid_clock::PacketGrid>, factor:f64) -> Result<(i64,i64)> {
        self.stamp_impl(source,frames,rate,tick,Some((grid,factor)))
    }
    fn stamp_impl(&mut self, source: i64, frames: usize, rate: u32, tick: u64,
        context:Option<(Option<fvid_media::owned_matroska_grid_clock::PacketGrid>,f64)>) -> Result<(i64,i64)> {
        let (exact_bracket,grid,factor)=context.map_or((false,None,1.0),|(grid,factor)|(true,grid,factor));
        let expected = self.origin.map(|origin| i128::from(origin)
            + i128::from(self.samples) * 1_000_000_000 / i128::from(rate));
        let matches = if exact_bracket {
            if let (Some(current),Some(epoch))=(grid,self.grid_origin) {
                fvid_media::owned_matroska_grid_clock::grid_sample_matches(current,epoch,self.samples,rate,tick,factor)
            } else {self.origin.is_some_and(|epoch| fvid_media::owned_matroska_sample_clock::quantized_sample_matches(
                source,epoch,self.samples,rate,tick))}
        } else { expected.is_some_and(|expected| (i128::from(source)-expected).unsigned_abs() <= u128::from(tick)) };
        let pts = if let Some(expected) = expected && matches {
            i64::try_from(expected).map_err(|_|invalid("MP3 sample clock overflow"))?
        } else { self.origin=Some(source);self.grid_origin=grid;self.samples=0;source };
        self.packet_start=self.samples;
        self.samples=self.samples.checked_add(frames as u64)
            .ok_or_else(||invalid("MP3 sample counter overflow"))?;
        let end=i128::from(self.origin.unwrap())
            + i128::from(self.samples)*1_000_000_000/i128::from(rate);
        Ok((pts,i64::try_from(end-i128::from(pts)).map_err(|_|invalid("MP3 duration overflow"))?))
    }
    fn exact_origin(&self,rate:u32,delay_ns:u64)->Option<i128> {
        if rate==0{return None;}
        let rate=i128::from(rate);let unit=1_000_000_000i128;
        let literal=i128::from(self.origin?)+i128::from(delay_ns);
        let projected=literal*rate;
        let nearest=(projected+unit/2).div_euclid(unit);
        let sample=nearest*unit;
        let lower=sample.div_euclid(rate);
        let upper=lower+i128::from(sample.rem_euclid(rate)!=0);
        // Only the two integer-ns representations of this exact sample point
        // qualify. Do not widen this to a container tick or round a seek floor.
        let origin=if literal==lower || literal==upper {sample}else{projected};
        let delay=(i128::from(delay_ns)*rate+unit/2).div_euclid(unit);
        Some(origin-delay*unit)
    }
    fn exact_skip(&self,floor:i64,rate:u32,frames:usize,delay_ns:u64)->Option<usize> {
        if rate==0{return None;}
        let origin=self.exact_origin(rate,delay_ns)?;
        let rate=i128::from(rate);let unit=1_000_000_000i128;
        let requested=i128::from(floor)*rate-origin;
        let target=requested.div_euclid(unit)+i128::from(requested.rem_euclid(unit)!=0);
        Some((target-i128::from(self.packet_start)).clamp(0,frames as i128)as usize)
    }
    fn exact_point(&self,first:usize,rate:u32,delay_ns:u64)->Option<u64> {
        if rate==0{return None;}
        let origin=self.exact_origin(rate,delay_ns)?;
        let rate=i128::from(rate);let unit=1_000_000_000i128;
        u64::try_from((origin+(i128::from(self.packet_start)+first as i128)*unit).div_euclid(rate)).ok()
    }
    /// Restore whole-sample CodecDelay from either adjacent integer ns before
    /// projecting PCM. Separately rounded packet/trim durations can drift by 1 ns.
    fn presented(&self, first:usize, rate:u32, delay_ns:u64)->Option<u64> {
        let delay=((u128::from(delay_ns)*u128::from(rate)+500_000_000)/1_000_000_000) as i128;
        let origin=i128::from(self.origin?)+i128::from(delay_ns);
        let samples=i128::from(self.packet_start)+first as i128-delay;
        let value=origin+samples*1_000_000_000/i128::from(rate);
        if value == -1 { Some(0) } else {u64::try_from(value).ok()}
    }
    fn skip_delayed(&self, floor:i64, rate:u32, frames:usize, delay_ns:u64)->usize {
        let delay=((u128::from(delay_ns)*u128::from(rate)+500_000_000)/1_000_000_000) as i128;
        let origin=i128::from(self.origin.unwrap_or(0))+i128::from(delay_ns);
        let numerator=(i128::from(floor)-origin)*i128::from(rate);
        let target=numerator.div_euclid(1_000_000_000)
            +i128::from(numerator.rem_euclid(1_000_000_000)!=0)+delay;
        (target-i128::from(self.packet_start)).max(0).min(frames as i128) as usize
    }

}

/// WebM audio track reader. Extracts encoded packets for decoding.
pub struct WebmAudioReader<R> {
    demuxer: WebmReader<R>,
    track_number: u64,
    in_band_ps: bool,
    packet_index: usize,
    presentation_floor: Option<i64>,
    mpeg_clock: MpegClock,
    ac3_clock: MpegClock,
    // Decode-order metadata outlives lookahead reads. Duplicate PTS remain
    // distinct entries; delayed E-AC-3 must not use the most recent clock.
    eac3_windows: std::cell::RefCell<std::collections::VecDeque<(i64,MpegClock,i64)>>,
    pcm_clock: MpegClock,
    flac_clock: MpegClock,
    alac_clock:std::cell::RefCell<MpegClock>,
    alac_end_ns:std::cell::Cell<Option<u64>>,
    alac_outputs:std::cell::Cell<usize>,
    alac_duration_complete:std::cell::Cell<bool>,
    wavpack_mask: Option<u32>,
    dolby_mask: Option<u32>,
    dolby_channels: Option<u16>,
    dolby_rate: Option<u32>,
    eac3_programme: u8,
    eac3_stereo_output: bool,
    wavpack_dsd: bool,
    flac_end_ns: Option<u64>,
    flac_duration_complete: bool,
    pcm_seek_error: Option<String>,
    aac_clock: std::cell::RefCell<MpegClock>,
    aac_end_ns: std::cell::Cell<Option<u64>>,
    aac_output_packets: std::cell::Cell<usize>,
    aac_duration_complete: std::cell::Cell<bool>,
    aac_clock_checkpoints: Vec<(i64, usize, MpegClock, usize, usize)>,
    aac_metadata_cursor: std::cell::Cell<usize>,
    aac_checkpoint_safe: std::cell::Cell<bool>,
    ac3_end_ns: std::cell::Cell<Option<u64>>,
    ac3_duration_complete: bool,
    // Match the worker's bounded one-per-second decoder checkpoint cache.
    mpeg_clock_checkpoints: Vec<(i64, usize, MpegClock)>,
    mpeg_checkpoint_safe: bool,
    /// The setup bytes the decoder asks for, in the layout it reads.
    extra_data: Vec<u8>,
    /// The name the decoder dispatch answers: the container's own tag for every
    /// coding but AAC, which dispatches as `mp4a` whichever container named it.
    codec_tag: String,
    discard_padding_ns: i64,
    opus_end_ns: Option<u64>,
    opus_clock: OpusClock,
    opus_duration_complete: bool,
    opus_indexed_end: std::sync::OnceLock<Option<u64>>,
    opus_packet_preskip: usize,
    vorbis: Option<VorbisClock>,
    // Final endpoint belongs to the track, not the rewindable decode clock.
    vorbis_end_ns: Option<u64>,
}

impl<R: Read + Seek> WebmAudioReader<R> {
    fn probe_eac3_programme(&mut self)->Result<()> {
        // Two complete programme cycles cover a packet boundary that starts
        // part-way through a multiplex. Each cycle has at most 8*(1+8) frames.
        const FRAMES:usize=2*8*9;
        const BYTES:usize=FRAMES*4096;
        let mut data=Vec::new();
        for _ in 0..=FRAMES {
            let packet=self.read_packet()?;let eof=packet.is_none();
            if let Some(packet)=packet {
                let available=BYTES.saturating_sub(data.len());
                data.extend_from_slice(&packet.data[..packet.data.len().min(available)]);
            }
            if let Some((frame,layout))=crate::codec::eac3_decoder::presentation::probe_programme(&data,self.eac3_programme,eof)? {
                self.dolby_rate=Some(frame.sample_rate);
                self.dolby_channels=Some(layout.0.len()as u16);
                self.dolby_mask=Some(layout.wave_mask().ok_or_else(||unsupported("E-AC-3 speaker locations need an extended speaker mask"))?);
                return Ok(());
            }
            if eof || data.len()==BYTES{break;}
        }
        Err(invalid("selected E-AC-3 programme is missing or incomplete in setup"))
    }

    fn eac3_speaker_mask(&mut self,first:&[u8])->Result<Option<u32>> {
        let channels=self.track().channels as u16;
        if channels<=6 {return Ok(Some(crate::codec::eac3_decoder::speaker_mask(first,channels)?));}
        let mut data=first.to_vec();
        for count in 0..9 {
            if let Some(layout)=crate::codec::eac3_decoder::presentation::try_packet_speakers(&data,channels)? {
                return layout.wave_mask().map(Some).ok_or_else(||unsupported("E-AC-3 speaker locations need an extended speaker mask"));
            }
            if count==8 {break;}
            let next=self.read_packet()?.ok_or_else(||invalid("incomplete E-AC-3 dependent presentation"))?;
            data.extend(next.data);
        }
        Err(invalid("E-AC-3 presentation exceeds eight dependent substreams"))
    }

    /// Prefer default usable audio; explicit lists retain container order.
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        let demuxer = WebmReader::open(reader, limits)?;
        let candidates = Self::supported(&demuxer);
        let usable: Vec<_> = candidates.iter().copied().filter(|number| {
            demuxer.tracks.iter().find(|track| track.number == *number)
                .is_some_and(Self::automatic_setup_usable)
        }).collect();
        // Preference applies after decoder admission. With no usable alternate,
        // retain the original candidate's detailed decoder refusal.
        let preferred = demuxer.preferred_track_numbers(if usable.is_empty() { &candidates } else { &usable });
        if let Some(number) = preferred.first().copied() {
            return Self::from_track(demuxer, number);
        }
        match demuxer
            .tracks
            .iter()
            .find(|t| t.kind == 2 && demuxer.track_enabled(t.number))
        {
            Some(track) => Err(unsupported(&format!(
                "WebM audio track coded `{}` has no decoder here",
                track.codec
            ))),
            None => Err(invalid("WebM has no such audio track")),
        }
    }

    /// Open the `nth` audio track this player can decode, counting in container
    /// order. Each one carries its own Vorbis setup headers, so the choice is
    /// made before the track is validated rather than after.
    pub fn open_at(reader: R, limits: Limits, nth: usize) -> Result<Self> {
        let demuxer = WebmReader::open(reader, limits)?;
        let number = match Self::supported(&demuxer).get(nth).copied() {
            Some(number) => number,
            // As in the MP4 reader: a listed audio track states its own coding, so
            // naming it separates a missing decoder arm from a container this reader
            // never got through.
            None => match demuxer
                .tracks
                .iter()
                .find(|t| t.kind == 2 && demuxer.track_enabled(t.number))
            {
                Some(track) => {
                    return Err(unsupported(&format!(
                        "WebM audio track coded `{}` has no decoder here",
                        track.codec
                    )));
                }
                None => return Err(invalid("WebM has no such audio track")),
            },
        };
        Self::from_track(demuxer, number)
    }

    /// Open an encoded E-AC-3 programme inside the zero-based physical audio track.
    /// Probe the chosen programme's geometry before creating a decoder. Rewind
    /// and seek retain this choice independently of the container's default track.
    pub fn open_programme(reader:R,limits:Limits,nth:usize,programme:u8)->Result<Self>{
        if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
        let demuxer=WebmReader::open(reader,limits)?;
        let number=Self::supported(&demuxer).get(nth).copied().ok_or_else(||invalid("WebM has no such audio track"))?;
        Self::from_programme(demuxer,number,Some(programme))
    }

    fn automatic_setup_usable(track: &Track) -> bool {
        let (Ok(rate), Ok(channels)) = (u32::try_from(track.sample_rate), u16::try_from(track.channels))
            else { return false; };
        if rate == 0 || channels == 0 { return false; }
        let Ok(extra) = setup_data(track) else { return false; };
        if track.codec == "A_AAC" {
            return crate::codec::config::aac_specific_config(&extra)
                .and_then(crate::codec::config::AudioSpecificConfig::parse).is_ok();
        }
        if track.codec == "A_VORBIS" {
            return crate::codec::make_audio_decoder(&track.codec, &extra, rate, channels, 0).is_ok();
        }
        if !matches!(track.codec.as_str(), "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE") {
            return true;
        }
        let Ok(bits) = u16::try_from(track.bit_depth) else { return false; };
        crate::codec::make_audio_decoder(&track.codec, &extra, rate, channels, bits).is_ok()
    }

    /// Project only known indexed packet durations; no payload reads or decode.
    fn indexed_opus_end_ns(&self) -> Option<u64> {
        let track = self.track();
        let preskip = u64::from(crate::codec::opus_decoder::header(&track.codec_private).ok()?.pre_skip);
        let delay = if track.codec_delay_ns != 0 { track.codec_delay_ns } else { preskip * 1_000_000_000 / 48000 };
        let delay = i64::try_from(delay).ok()?;
        let mut clock = OpusClock::default(); let mut endpoint = 0;
        for (index,packet) in self.demuxer.packets.iter().enumerate().filter(|(_,p)| p.track == self.track_number) {
            let duration = packet.duration_ns.filter(|&duration| duration != 0)
                .or_else(|| (track.default_duration_ns != 0).then_some(track.default_duration_ns))?;
            let pts = packet.pts_ns.checked_sub(delay)?;
            let timing = clock.stamp(pts, duration, preskip, self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(index),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0),delay as u64).ok()?;
            if let Some(end) = timing.audible_end(packet.discard_padding_ns).ok()? { endpoint = endpoint.max(end); }
        }
        Some(endpoint)
    }

    /// Track numbers of the audio tracks a decoder exists for, in container order.
    fn supported(demuxer: &WebmReader<R>) -> Vec<u64> {
        demuxer
            .tracks
            .iter()
            .filter(|t| {
                t.kind == 2 && demuxer.track_enabled(t.number) && CODECS.contains(&t.codec.as_str())
            })
            .map(|t| t.number)
            .collect()
    }

    /// Open a track the demuxer already listed.
    fn from_track(demuxer:WebmReader<R>,track_number:u64)->Result<Self>{Self::from_programme(demuxer,track_number,None)}
    fn from_programme(mut demuxer: WebmReader<R>, track_number: u64,programme:Option<u8>) -> Result<Self> {
        let track = demuxer
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| invalid("WebM has no such audio track"))?;
        let extra_data = setup_data(track)?;
        let codec_tag = if track.codec == "A_AAC" {
            "mp4a".to_string()
        } else if matches!(track.codec.as_str(), "A_AC3/BSID9" | "A_AC3/BSID10") {
            "A_AC3".to_string()
        } else {
            track.codec.clone()
        };
        if programme.is_some() && codec_tag!="A_EAC3"{return Err(unsupported("programme selection requires an E-AC-3 track"));}
        if programme.is_none() && (track.sample_rate == 0 || track.sample_rate > u64::from(u32::MAX)) {
            return Err(invalid("WebM audio track has no usable sample rate"));
        }
        if programme.is_none() && (track.channels == 0 || track.channels > u64::from(u16::MAX)) {
            return Err(invalid("WebM audio track has no usable channel count"));
        }
        let vorbis = if codec_tag == "A_VORBIS" {
            let (ident, setup) = fvid_codecs::vorbis_audio::configuration_headers(&extra_data)?;
            let ident = fvid_codecs::vorbis::Identification::parse(ident)?;
            if u64::from(ident.sample_rate) != track.sample_rate
                || u64::from(ident.channels) != track.channels
            {
                return Err(invalid("Vorbis track/header geometry mismatch"));
            }
            let setup = fvid_codecs::vorbis::Setup::parse(
                setup,
                ident.channels,
                fvid_codecs::vorbis_admission::inspect(setup)?.limits,
            )?;
            Some(VorbisClock {
                blocks: ident.block_exponents,
                modes: setup.modes,
                previous: None,
                origin: None,
                samples: 0,
                end_ns: None,
            })
        } else {
            None
        };
        let candidate = if codec_tag == "mp4a" {
            let asc = crate::codec::config::aac_specific_config(&extra_data)?;
            let config = crate::codec::config::AudioSpecificConfig::parse(asc)?;
            if config.ps_present.is_none() && config.sbr_present != Some(false) {
                crate::codec::aac_ps_native::InBandPsProbe::new(asc, track.sample_rate as u32).ok()
            } else {
                None
            }
        } else {
            None
        };
        let mut in_band_ps = false;
        if let Some(mut probe) = candidate {
            let mut index = 0;
            while index < demuxer.packets.len() || demuxer.scan_more()? {
                if demuxer.packets[index].track == track_number {
                    let payload = demuxer.read_packet(index)?;
                    if probe.read(&payload).is_err() {
                        probe.reset();
                    } else if probe.ps_detected() {
                        in_band_ps = true;
                        break;
                    }
                }
                index += 1;
            }
        }
        let mut source=Self {
            in_band_ps,
            demuxer,
            track_number,
            packet_index: 0,
            presentation_floor: None,
            mpeg_clock: MpegClock::default(),
            ac3_clock: MpegClock::default(),
            eac3_windows:Default::default(),
            pcm_clock: MpegClock::default(),
            flac_clock: MpegClock::default(),
            alac_clock:std::cell::RefCell::new(MpegClock::default()),
            alac_end_ns:std::cell::Cell::new(None),
            alac_outputs:std::cell::Cell::new(0),
            alac_duration_complete:std::cell::Cell::new(false),
            wavpack_mask: None,
            dolby_mask: None,
            dolby_channels: None,
            dolby_rate:None,
            eac3_programme:programme.unwrap_or(0),
            eac3_stereo_output: false,
            wavpack_dsd: false,
            flac_end_ns: None,
            flac_duration_complete: false,
            pcm_seek_error: None,
            aac_clock: std::cell::RefCell::new(MpegClock::default()),
            aac_end_ns: std::cell::Cell::new(None),
            aac_output_packets: std::cell::Cell::new(0),
            aac_duration_complete: std::cell::Cell::new(false),
            aac_clock_checkpoints: Vec::new(),
            aac_metadata_cursor: std::cell::Cell::new(0),
            aac_checkpoint_safe: std::cell::Cell::new(true),
            ac3_end_ns: std::cell::Cell::new(None),
            ac3_duration_complete: false,
            mpeg_clock_checkpoints: Vec::new(),
            mpeg_checkpoint_safe: true,
            extra_data,
            codec_tag,
            discard_padding_ns: 0,
            opus_end_ns: None,
            opus_clock: OpusClock::default(),
            opus_duration_complete: false,
            opus_indexed_end: std::sync::OnceLock::new(),
            opus_packet_preskip: 0,
            vorbis,
            vorbis_end_ns: None,
        };
        if programme.is_some(){
            source.probe_eac3_programme()?;
            source.rewind();
        }else if matches!(source.codec_tag.as_str(),"A_AC3"|"A_EAC3") {
            if let Some(packet)=source.read_packet()? {
                // An unreadable first header retains the decoder's existing late
                // refusal/concealment behavior; never invent a mask from width.
                source.dolby_mask = if source.codec_tag=="A_AC3" {
                    if let Some(frame)=crate::codec::ac3_decoder::checked_syncframe(&packet.data) {
                        source.dolby_channels=Some(frame.channels);
                        Some(crate::codec::ac3_decoder::speaker_mask(&packet.data,frame.channels)?)
                    }else{None}
                }else if crate::codec::eac3_decoder::checked_frame(&packet.data).is_some() {
                    source.eac3_speaker_mask(&packet.data)?
                }else{None};
            }
            source.rewind();
        }
        if source.codec_tag=="A_WAVPACK4" {
            if let Some(packet)=source.read_packet()? {
                source.wavpack_dsd=crate::codec::wavpack_decoder::WavpackDecoder::matroska_is_dsd(
                    &source.extra_data,&packet.data,source.track().sample_rate as u32,source.track().channels as u16)?;
                source.wavpack_mask=crate::codec::wavpack_decoder::WavpackDecoder::matroska_speaker_mask(
                    &source.extra_data,&packet.data,source.track().sample_rate as u32,source.track().channels as u16)?;
            }
            source.rewind();
        }
        if source.codec_tag=="A_FLAC" {
            // Index actual coded sample counts once. No PCM reconstruction or
            // duplicate packet index is needed; rewind preserves the endpoint.
            while let Some(mut packet)=source.read_packet()? {source.stamp_flac_packet(&mut packet)?;}
            source.flac_duration_complete=true;
            source.rewind();
        }
        if source.codec_tag=="A_ALAC" {
            while let Some(packet)=source.read_packet()? {
                let frames=source.alac_packet_samples(&packet.data)?;
                source.stamp_alac_geometry(packet.pts_ns,frames)?;
            }
            source.alac_duration_complete.set(true);
            source.rewind();
        }
        Ok(source)
    }

    fn alac_packet_samples(&self,data:&[u8])->Result<usize>{
        crate::codec::alac_decoder::AlacDecoder::packet_samples(&self.extra_data,u32::try_from(self.track().sample_rate).map_err(|_|invalid("ALAC sample rate overflow"))?,u16::try_from(self.track().channels).map_err(|_|invalid("ALAC channel count overflow"))?,data)
    }

    fn seek_alac(&mut self,target:i64)->Result<()> {
        let rate=u32::try_from(self.track().sample_rate).map_err(|_|invalid("ALAC sample rate overflow"))?;
        loop {
            let before=self.alac_clock.borrow().clone();
            let outputs=self.alac_outputs.get();
            let Some(packet)=self.read_packet()? else{return Ok(());};
            let frames=self.alac_packet_samples(&packet.data)?;
            self.stamp_alac_geometry(packet.pts_ns,frames)?;
            let padding=discard_padding_samples(self.discard_padding_ns,rate)as usize;
            let first=self.alac_clock.borrow().exact_skip(target,rate,frames,self.index_delay_ns()).ok_or_else(||invalid("invalid ALAC seek sample clock"))?.max(if self.discard_padding_ns<0{padding}else{0});
            let last=frames-if self.discard_padding_ns>0{padding}else{0};
            if first<last {
                self.packet_index-=1;
                *self.alac_clock.get_mut()=before;
                self.alac_outputs.set(outputs);
                return Ok(());
            }
        }
    }

    fn stamp_alac_geometry(&self,source_pts:i64,frames:usize)->Result<i64>{
        let rate=u32::try_from(self.track().sample_rate).map_err(|_|invalid("ALAC sample rate overflow"))?;
        let index=self.packet_index.checked_sub(1).ok_or_else(||invalid("missing ALAC source packet"))?;
        let mut clock=self.alac_clock.borrow_mut();
        let pts=clock.stamp_aac(source_pts,frames,rate,self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(index),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0))?.0;
        let padding=discard_padding_samples(self.discard_padding_ns,rate);
        if padding>frames as u128 || discard_padding_exceeds_packet(self.discard_padding_ns,rate,frames){return Err(invalid("Matroska discard padding exceeds decoded packet"));}
        let padding=padding as usize;
        let first=clock.exact_skip(0,rate,frames,self.index_delay_ns()).ok_or_else(||invalid("invalid ALAC sample clock"))?.max(if self.discard_padding_ns<0{padding}else{0});
        let last=frames-if self.discard_padding_ns>0{padding}else{0};
        if first<last {
            let end=clock.exact_point(last,rate,self.index_delay_ns()).ok_or_else(||invalid("negative ALAC presentation end"))?;
            self.alac_end_ns.set(Some(self.alac_end_ns.get().unwrap_or(0).max(end)));
        }
        self.alac_outputs.set(self.alac_outputs.get().checked_add(1).ok_or_else(||invalid("ALAC output packet counter overflow"))?);
        Ok(pts)
    }

    fn stamp_flac_packet(&mut self,p:&mut AudioPacket)->Result<i64> {
            // STREAMINFO, rather than optional Matroska BitDepth, is authoritative.
            let info=flac_stream_info(&self.track().codec_private);
            if info.len()!=34{return Err(invalid("FLAC requires STREAMINFO"));}
            let word=u64::from_be_bytes(info[10..18].try_into().unwrap());
            let config=fvid_codecs::flac::Config{sample_rate:u32::try_from(self.track().sample_rate).map_err(|_|invalid("FLAC sample rate overflow"))?,channels:u16::try_from(self.track().channels).map_err(|_|invalid("FLAC channel count overflow"))?,bits:((word>>36)&31)as u8+1};
            let frames=fvid_codecs::flac::frame_samples(&p.data,config)?;
            let (pts,duration)=self.flac_clock.stamp_aac(p.pts_ns,frames,config.sample_rate,self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(self.packet_index-1),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0))?;
            let rate=config.sample_rate;
            let padding=discard_padding_samples(self.discard_padding_ns,rate);
            if padding>frames as u128 || discard_padding_exceeds_packet(self.discard_padding_ns,rate,frames){return Err(invalid("Matroska discard padding exceeds decoded packet"));}
            let first=self.flac_clock.exact_skip(0,rate,frames,self.index_delay_ns()).ok_or_else(||invalid("invalid FLAC sample clock"))?
                .max(if self.discard_padding_ns<0{padding as usize}else{0});
            let last=frames-if self.discard_padding_ns>0{padding as usize}else{0};
            if first<last {
                let end=self.flac_clock.exact_point(last,rate,self.index_delay_ns()).ok_or_else(||invalid("negative FLAC presentation end"))?;
                self.flac_end_ns=Some(self.flac_end_ns.unwrap_or(0).max(end));
            }
            p.pts_ns=pts;Ok(duration)
    }

    /// Read the next audio packet. Returns None at end of track.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        if let Some(error)=self.pcm_seek_error.take(){return Err(invalid(&error));}
        // Blocks are indexed a cluster at a time, so running out of them asks
        // the file for more rather than meaning the track has ended.
        while self.packet_index < self.demuxer.packets.len() || self.demuxer.scan_more()? {
            let idx = self.packet_index;
            let (pts_ns, track) = {
                let packet = &self.demuxer.packets[idx];
                (packet.pts_ns, packet.track)
            };
            self.packet_index += 1;

            if track != self.track_number {
                continue;
            }

            let mut data = self.demuxer.read_packet(idx)?;
            if self.codec_tag=="A_WAVPACK4" && let Some(extra)=self.demuxer.read_block_additional(idx,1)? {
                data=crate::codec::wavpack_decoder::WavpackDecoder::matroska_with_correction(
                    &self.extra_data,self.track().sample_rate as u32,self.track().channels as u16,&data,&extra)?;
            }
            self.discard_padding_ns = self.demuxer.packets[idx].discard_padding_ns;
            let pts_ns = if self.codec_tag == "A_OPUS" {
                let delay_ns = if self.track().codec_delay_ns != 0 {
                    self.track().codec_delay_ns
                } else {
                    u64::from(
                        crate::codec::opus_decoder::header(&self.track().codec_private)?.pre_skip,
                    ) * 1_000_000_000
                        / 48000
                };
                let delay =
                    i64::try_from(delay_ns).map_err(|_| invalid("Opus codec delay overflow"))?;
                pts_ns
                    .checked_sub(delay)
                    .ok_or_else(|| invalid("Opus timestamp overflow"))?
            } else if self.codec_tag == "A_VORBIS" {
                pts_ns
                    .checked_sub(
                        i64::try_from(self.track().codec_delay_ns)
                            .map_err(|_| invalid("Vorbis codec delay overflow"))?,
                    )
                    .ok_or_else(|| invalid("Vorbis codec delay timestamp overflow"))?
            } else {
                self.indexed_timestamp(pts_ns)
                    .ok_or_else(|| invalid("audio codec delay timestamp overflow"))?
            };

            return Ok(Some(AudioPacket {
                data,
                pts_ns,
                track,
            }));
        }
        Ok(None)
    }

    /// The audio track metadata.
    pub fn track(&self) -> &Track {
        self.demuxer
            .tracks
            .iter()
            .find(|t| t.number == self.track_number)
            .expect("audio track must exist")
    }

    fn pcm_geometry(&self)->Result<(usize,u32)> {
        let track=self.track();
        let rate=u32::try_from(track.sample_rate).map_err(|_|invalid("PCM sample rate overflow"))?;
        let channels=u16::try_from(track.channels).map_err(|_|invalid("PCM channel count overflow"))?;
        let bits=u16::try_from(track.bit_depth).map_err(|_|invalid("PCM bit depth overflow"))?;
        let spec=if self.codec_tag=="A_PCM/FLOAT/IEEE" {
            crate::codec::pcm_decoder::PcmDecoder::float(bits,rate,channels)?
        }else{crate::codec::pcm_decoder::PcmDecoder::int(bits,self.codec_tag=="A_PCM/INT/BIG",rate,channels)?};
        Ok((spec.encoded_frame_bytes()?,rate))
    }
    fn pcm_packet_frames(&self,index:usize,stride:usize)->Result<usize> {
        let bytes=self.demuxer.packet_payload_size(index)?;
        if stride==0||!bytes.is_multiple_of(stride){return Err(invalid("PCM packet ends in an incomplete channel frame"));}
        Ok(bytes/stride)
    }
    fn stamp_pcm(&self,clock:&mut MpegClock,index:usize,frames:usize,rate:u32)->Result<i64> {
        let source=self.indexed_timestamp(self.demuxer.packets[index].pts_ns)
            .ok_or_else(||invalid("PCM codec delay timestamp overflow"))?;
        let (at,_)=clock.stamp_aac(source,frames,rate,self.demuxer.timestamp_scale_ns(),
            self.demuxer.packet_timestamp_grid(index),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0))?;
        Ok(at)
    }
    fn pcm_bounds(&self,index:usize,frames:usize,clock:&MpegClock,rate:u32,floor:i64)->Result<(usize,usize)> {
        let ns=self.demuxer.packets[index].discard_padding_ns;
        let padding=discard_padding_samples(ns,rate);
        if padding>frames as u128||discard_padding_exceeds_packet(ns,rate,frames){return Err(invalid("Matroska discard padding exceeds PCM packet"));}
        let skip=clock.exact_skip(floor.max(0),rate,frames,self.index_delay_ns())
            .ok_or_else(||invalid("invalid PCM sample clock"))?;
        let first=skip.max(if ns<0 {padding as usize}else{0});
        let last=frames-if ns>0{padding as usize}else{0};
        Ok((first,last))
    }
    fn seek_pcm(&mut self,target:i64)->Result<()> {
        let (stride,rate)=self.pcm_geometry()?;let mut index=0;let mut clock=MpegClock::default();
        loop {
            while index<self.demuxer.packets.len() {
                if self.demuxer.packets[index].track!=self.track_number{index+=1;continue;}
                let frames=self.pcm_packet_frames(index,stride)?;let before=clock.clone();
                self.stamp_pcm(&mut clock,index,frames,rate)?;
                let(first,last)=self.pcm_bounds(index,frames,&clock,rate,target)?;
                if first<last {self.packet_index=index;self.pcm_clock=before;return Ok(());}
                index+=1;
            }
            if !self.demuxer.scan_more()? {self.packet_index=self.demuxer.packets.len();self.pcm_clock=clock;return Ok(());}
        }
    }
    fn pcm_duration(&self)->Option<std::time::Duration> {
        let(stride,rate)=self.pcm_geometry().ok()?;let mut clock=MpegClock::default();let mut end=0;let mut seen_packet=false;
        for index in 0..self.demuxer.packets.len() {
            if self.demuxer.packets[index].track!=self.track_number{continue;}
            let frames=self.pcm_packet_frames(index,stride).ok()?;self.stamp_pcm(&mut clock,index,frames,rate).ok()?;
            let(first,last)=self.pcm_bounds(index,frames,&clock,rate,0).ok()?;
            // A fully indexed, valid track whose windows all trim away has a
            // known zero endpoint, distinct from an absent/invalid packet clock.
            seen_packet=true;
            if first<last {end=end.max(clock.exact_point(last,rate,self.index_delay_ns())?);}
        }
        seen_packet.then(||std::time::Duration::from_nanos(end))
    }

    /// Restart from the first packet.
    pub fn rewind(&mut self) {
        self.packet_index = 0;
        self.presentation_floor = None;
        self.mpeg_clock = MpegClock::default();
        self.ac3_clock = MpegClock::default();
        self.eac3_windows.get_mut().clear();
        self.pcm_clock = MpegClock::default();
        self.flac_clock = MpegClock::default();
        *self.alac_clock.get_mut()=MpegClock::default();
        self.alac_outputs.set(0);
        self.pcm_seek_error = None;
        *self.aac_clock.get_mut() = MpegClock::default();
        self.aac_output_packets.set(0);
        self.aac_metadata_cursor.set(0);
        self.opus_clock = OpusClock::default();
        self.opus_packet_preskip = 0;
        if let Some(clock) = &mut self.vorbis {
            clock.reset();
        }
    }

    // Opus keeps its historical packet-index coordinates; its stream seek
    // rebuilds the codec's preskip history. Other codecs index actual playback
    // timestamps, including a track's explicitly declared CodecDelay.
    fn index_delay_ns(&self) -> u64 {
        if self.codec_tag == "A_OPUS" { 0 } else { self.track().codec_delay_ns }
    }
    fn indexed_timestamp(&self, source: i64) -> Option<i64> {
        i64::try_from(i128::from(source) - i128::from(self.index_delay_ns())).ok()
    }

    /// Seek to the packet with the greatest PTS at or before `pts_ns`.
    /// Returns that packet's PTS, or the first one when the whole track follows.
    pub fn seek(&mut self, pts_ns: i64) -> i64 {
        // A jump needs the blocks behind its target, which an index that grew a
        // cluster at a time has not necessarily reached yet. The infallible
        // signature is the audio stream's own; a walk that fails leaves the
        // index short, and the next packet read reports the same failure where
        // the caller can still hear it.
        let delay = self.index_delay_ns();
        let source_target = (i128::from(pts_ns) + i128::from(delay))
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        let _ = self.demuxer.scan_until(source_target);
        let mut best: Option<(usize, i64)> = None;
        let mut first_on_track: Option<(usize, i64)> = None;
        loop {
            for (i, packet) in self.demuxer.packets.iter().enumerate() {
                if packet.track != self.track_number { continue; }
                let Ok(at) = i64::try_from(i128::from(packet.pts_ns) - i128::from(delay)) else { continue; };
                if first_on_track.is_none() {
                    first_on_track = Some((i, at));
                }
                if at > pts_ns { continue; }
                if best.is_none_or(|(_, previous)| at > previous) {
                    best = Some((i, at));
                }
            }
            if best.is_some() || first_on_track.is_some() {
                break;
            }
            let Ok(more) = self.demuxer.scan_more() else { break; };
            if !more {
                break;
            }
        }
        let (index, landed) = best.or(first_on_track).unwrap_or((0, 0));
        self.packet_index = index;
        landed
    }
}

/// Vorbis stores its three header packets in `CodecPrivate`, but a decoder only
/// needs the identification and setup ones. Writers disagree about whether
/// those packets carry Xiph lacing, and symphonia unpacks the laced form
/// itself, so only the raw concatenation is rewritten here.
fn vorbis_setup_headers(private: &[u8]) -> Result<Vec<u8>> {
    const IDENT_TYPE: u8 = 1;
    const COMMENT: &[u8; 7] = b"\x03vorbis";
    const SETUP: &[u8; 7] = b"\x05vorbis";

    if private.first() != Some(&IDENT_TYPE) {
        return Ok(private.to_vec());
    }
    let find = |marker: &[u8; 7], from: usize| {
        private[from..]
            .windows(marker.len())
            .position(|w| w == marker)
            .map(|at| at + from)
    };
    let comment = find(COMMENT, 1).ok_or_else(|| invalid("WebM Vorbis headers are truncated"))?;
    let setup =
        find(SETUP, comment).ok_or_else(|| invalid("WebM Vorbis headers have no setup header"))?;
    let mut headers = private[..comment].to_vec();
    headers.extend_from_slice(&private[setup..]);
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::{WebmAudioReader, aac_setup, flac_stream_info, vorbis_setup_headers};
    use crate::audio::{AudioStream, AudioTrack};
    use crate::container::webm::Limits;
    use std::io::Cursor;

    fn header(kind: u8, tail: &[u8]) -> Vec<u8> {
        let mut v = vec![kind, b'v', b'o', b'r', b'b', b'i', b's'];
        v.extend_from_slice(tail);
        v
    }

    /// The container lists the tracks a decoder exists for, so the player's
    /// track key knows when there is nothing to choose. A request past the end
    /// is an error rather than a silent fallback to the first track.
    #[test]
    fn the_fixture_lists_its_audio_tracks() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
        assert_eq!(
            stream.audio_tracks(),
            vec![AudioTrack {
                sample_rate: 44_100,
                channels: 2,
                name: String::new(),
                language: String::new(),
            }]
        );
        let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 0).unwrap();
        assert_eq!((chosen.sample_rate(), chosen.channels()), (44_100, 2));
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).is_err());
    }

    /// A file that says nothing about a track leaves the layout as its whole
    /// name; a title or a language is said first, and the layout follows it
    /// because two tracks can share a name and still differ in what they hold.
    #[test]
    fn a_track_is_named_by_what_the_file_says_and_what_it_holds() {
        let said = |name: &str, language: &str| {
            AudioTrack {
                sample_rate: 48_000,
                channels: 6,
                name: name.to_owned(),
                language: language.to_owned(),
            }
            .label()
        };
        assert_eq!(said("", ""), "6 ch 48000 Hz");
        assert_eq!(said("", "eng"), "eng · 6 ch 48000 Hz");
        assert_eq!(said("Surround", ""), "Surround · 6 ch 48000 Hz");
        assert_eq!(said("Surround", "eng"), "Surround · 6 ch 48000 Hz");
    }

    /// The same list out of a Matroska file: the muxer wrote the title into the
    /// `Name` element and the language into the one beside it, and the reader
    /// reaches both. `tests/fixtures/tracks/named.mkv`, whose command the
    /// container's own test records.
    #[test]
    fn a_named_file_lists_its_audio_tracks_by_what_it_says() {
        const NAMED: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mkv");
        let stream = WebmAudioReader::open(Cursor::new(NAMED), Limits::default()).unwrap();
        let labels: Vec<String> = stream
            .audio_tracks()
            .iter()
            .map(AudioTrack::label)
            .collect();
        assert_eq!(labels, ["Первая · 1 ch 48000 Hz", "fre · 1 ch 32000 Hz"]);
    }

    #[test]
    fn raw_headers_drop_the_comment_packet() {
        let mut private = header(1, b"id");
        private.extend_from_slice(&header(3, b"comment"));
        private.extend_from_slice(&header(5, b"setup"));
        let mut expected = header(1, b"id");
        expected.extend_from_slice(&header(5, b"setup"));
        assert_eq!(vorbis_setup_headers(&private).unwrap(), expected);
    }

    #[test]
    fn laced_headers_pass_through_untouched() {
        let private = vec![2, 3, 7, 1, b'v', b'o', b'r', b'b', b'i', b's'];
        assert_eq!(vorbis_setup_headers(&private).unwrap(), private);
    }

    #[test]
    fn missing_setup_header_is_an_error() {
        let private = header(1, b"id");
        assert!(vorbis_setup_headers(&private).is_err());
    }

    /// The whole WebM audio path over a real file: demux, rebuild the setup
    /// headers, decode. The fixture is 2 s of 440 Hz sine recorded at −20.9 dBFS,
    /// which is what ffmpeg's own `volumedetect` reports for it, so the decoded
    /// peak is checked against that rather than against full scale.
    #[test]
    fn vorbis_fixture_decodes_to_pcm() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a decodable audio track");
        assert_eq!(stream.codec(), "A_VORBIS");
        assert_eq!(stream.timescale(), 1_000_000_000);
        assert_eq!(stream.sample_rate(), 44_100);
        assert_eq!(stream.channels(), 2);
        let extra = stream.extra_data();
        // ffmpeg stores the headers laced, so the first byte is a lace count and
        // the decoder unpacks them; only the bare concatenation gets rewritten.
        assert_eq!(extra, stream.track().codec_private);
        for marker in [b"\x01vorbis" as &[u8], b"\x03vorbis", b"\x05vorbis"] {
            assert!(extra.windows(7).any(|w| w == marker));
        }

        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("Vorbis decoder");
        let mut frames = 0usize;
        let mut peak = 0.0f32;
        let bytes = bytes_per_frame(stream.channels());
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 44_100));
            assert_eq!(pcm.data.len() % bytes, 0);
            frames += pcm.data.len() / bytes;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        // The 88 packets' block sizes sum to exactly 2.02 s. Total rather than a
        // range: a decoder that swallowed or doubled one packet would still land
        // inside any plausible band.
        assert_eq!(frames, 89_088);
        // 10^(-20.9/20) = 0.090, to within the encoder's own rounding.
        assert!(
            (0.088..=0.092).contains(&peak),
            "peak={peak} against ffmpeg's -20.9 dBFS"
        );

        // The same packets again, from the cursor's own rewind.
        stream.rewind();
        decoder.reset();
        let mut again = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            if let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            {
                again += pcm.data.len() / bytes;
            }
        }
        assert_eq!(again, frames);
    }

    /// Decode every packet of a fixture and report how much sound came out.
    fn decode_all(stream: &mut WebmAudioReader<Cursor<&'static [u8]>>) -> (usize, f32) {
        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("decoder");
        let rate = stream.sample_rate();
        let bytes = bytes_per_frame(stream.channels());
        let mut frames = 0usize;
        let mut peak = 0.0f32;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
            assert_eq!(pcm.data.len() % bytes, 0);
            frames += pcm.data.len() / bytes;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        (frames, peak)
    }

    /// The FLAC fixture is 2 s of a 0.3-amplitude stereo sine, made with:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=2:s=44100' \
    ///   -vn -c:a flac tests/fixtures/audio/flac-stereo.mkv
    /// The coding is lossless, so both the frame count and the peak are exact
    /// and match what ffmpeg's own decode of the same file reports.
    #[test]
    fn a_flac_track_decodes_to_its_exact_length() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a FLAC track");
        assert_eq!(stream.codec(), "A_FLAC");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        // Of the 42 setup bytes the muxer stored, only the 34-byte STREAMINFO
        // body reaches the decoder.
        let private = stream.track().codec_private.clone();
        assert_eq!(stream.extra_data().len(), 34);
        assert_eq!(stream.extra_data(), flac_stream_info(&private));
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 88_200);
        assert!((0.2999..=0.3001).contains(&peak), "peak={peak}");
    }

    /// The MP3 fixture is the same sine at 128 kbit/s:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=2:s=44100' \
    ///   -vn -c:a libmp3lame -b:a 128k tests/fixtures/audio/mp3-stereo.mkv
    /// One frame is 1152 samples and the encoder puts its delay in front of the
    /// first packet, so the total is the packet count times 1152 rather than the
    /// nominal two seconds. The peak is a band: lossy coding decides it, and
    /// ffmpeg's own decode of this file peaks at 0.291.
    #[test]
    fn an_mp3_track_decodes_frame_by_frame() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/mp3-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an MP3 track");
        assert_eq!(stream.codec(), "A_MPEG/L3");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        assert!(stream.extra_data().is_empty(), "a frame says all of it");
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 78 * 1_152);
        assert!((0.28..=0.30).contains(&peak), "peak={peak}");
    }

    /// Both Opus and FLAC are now offered in container order.
    #[test]
    fn the_track_list_includes_opus_alongside_flac() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/opus-flac.mkv");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
        assert_eq!(stream.audio_tracks().len(), 2);
        assert_eq!(stream.codec(), "A_OPUS");
        assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
        let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).unwrap();
        assert_eq!(chosen.codec(), "A_FLAC");
        assert_eq!((chosen.sample_rate(), chosen.channels()), (44100, 2));
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 2).is_err());
    }

    #[test]
    fn the_track_list_still_skips_unknown_audio() {
        let mut bytes = include_bytes!("../../../tests/fixtures/audio/opus-flac.mkv").to_vec();
        let at = bytes.windows(6).position(|p| p == b"A_OPUS").unwrap();
        bytes[at..at + 6].copy_from_slice(b"A_NOPE");
        let stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
        assert_eq!(stream.audio_tracks().len(), 1);
        assert_eq!(stream.codec(), "A_FLAC");
    }

    /// Three audio tracks from one source, every one of them decodable:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=44100' \
    ///   -vn -map 0:a -c:a:0 ac3 -b:a:0 96k -map 0:a -c:a:1 flac \
    ///   -map 0:a -c:a:2 libmp3lame -b:a:2 96k tests/fixtures/audio/ac3-flac-mp3.mkv
    /// The Dolby track used to be the one this list dropped. It is here now, in
    /// the order the file gives, and only a key past all three is an error.
    #[test]
    fn a_dolby_track_is_listed_with_the_lossy_ones_it_sits_beside() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-flac-mp3.mkv");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has audio tracks");
        assert_eq!(stream.audio_tracks().len(), 3);
        for (nth, codec) in [(0, "A_AC3"), (1, "A_FLAC"), (2, "A_MPEG/L3")] {
            let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), nth)
                .unwrap_or_else(|error| panic!("track {nth}: {error}"));
            assert_eq!(chosen.codec(), codec);
        }
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 3).is_err());
    }

    /// One second of two sines coded as AAC and copied into Matroska, which is
    /// what a remux of an `.m4a` leaves behind:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(1100*PI*t):d=1:s=48000' \
    ///   -c:a aac -b:a 128k /tmp/aac-src.m4a
    /// ffmpeg -i /tmp/aac-src.m4a -vn -c copy tests/fixtures/audio/aac-stereo.mka
    /// ```
    /// The track names itself `A_AAC` and carries its setup as the bare
    /// AudioSpecificConfig — five bytes, which `ffprobe` reports as the file's
    /// `extradata_size`. The decoder asks for an MP4 `esds`, so the wrapper is
    /// rebuilt, and the dispatch answers this coding as `mp4a` whatever the
    /// container called it.
    #[test]
    fn an_aac_track_arrives_with_only_the_config_and_is_wrapped() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/aac-stereo.mka");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an AAC track");
        assert_eq!(stream.codec(), "mp4a");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        assert_eq!(
            crate::codec::config::aac_specific_config(stream.extra_data())
                .expect("a setup block the decoder's own parser accepts")
                .len(),
            5,
            "the whole config the container handed over, not a truncated copy"
        );
        let (frames, peak) = decode_all(&mut stream);
        // 48 kHz AAC blocks are 1024 samples; a second of sound is 47 of them and
        // the decoder hands over the priming frame the encoder put in front.
        assert!(
            (47 * 1024..49 * 1024).contains(&frames),
            "decoded {frames} frames of a one second track"
        );
        // The loudest sample this route produces is the one ffmpeg's own reading
        // of the same stream produces, to the last bit: 0.41567978. It is well
        // above the 0.3 the sine was written at, and that overshoot belongs to
        // the coding rather than to this decode, which is why the number is
        // checked against the reference instead of against the amplitude.
        assert!(
            (peak - 0.415_679_78).abs() < 5e-5,
            "peak={peak}, the reference reads 0.41567978"
        );
    }

    /// A setup block that cannot name a coding is refused with the length it
    /// states, so a file with an empty `CodecPrivate` does not reach the decoder
    /// and fail there instead.
    #[test]
    fn an_aac_track_without_a_usable_setup_block_is_refused_by_name() {
        let error = aac_setup(&[0x11]).expect_err("one byte states no coding");
        assert!(
            error.to_string().contains("1 bytes"),
            "the refusal quotes the setup length: {error}"
        );
        assert!(aac_setup(&[0x11, 0x90]).is_ok(), "two bytes is enough");
    }

    /// The coding DVB and broadcast rips keep in Matroska, one second of a 440 Hz
    /// sine in two channels:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=48000' \
    ///   -vn -c:a mp2 -b:a 256k tests/fixtures/audio/mp2-stereo.mkv
    /// ```
    /// MPEG Layer II has had a decoder arm here for a while, because a bare
    /// `.mp2` file names itself with this very tag; what it never had was a
    /// container route, so a track Matroska called `A_MPEG/L2` was dropped from
    /// the list before the dispatch was ever asked.
    #[test]
    fn a_layer_ii_track_reaches_the_arm_the_layer_iii_route_shares() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/mp2-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an MP2 track");
        assert_eq!(stream.codec(), "A_MPEG/L2");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        let (frames, peak) = decode_all(&mut stream);
        // Layer II blocks are 1152 samples at this rate, so a second is 41 of them
        // and change; the reference reads 47 903 frames after its own 481-sample
        // priming, and this route keeps the priming it is handed, so the count is
        // bounded rather than pinned.
        assert!(
            (41 * 1152..43 * 1152).contains(&frames),
            "decoded {frames} frames of a one second track"
        );
        // Both channels carry the same tone, and this is the number ffmpeg's
        // reading of the same stream gives.
        assert!(
            (peak - 0.301_391_6).abs() < 5e-5,
            "peak={peak}, the reference reads 0.3013916"
        );
    }

    /// One quarter second of the 0.3-amplitude stereo sine, written as the lossless
    /// integer PCM Matroska calls `A_PCM/INT/LIT`:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=0.25:s=44100' \
    ///   -vn -c:a pcm_s24le tests/fixtures/audio/pcm-int.mkv
    /// ```
    /// Eleven blocks of 6144 bytes, so 11025 frames of three little-endian bytes
    /// per channel. `BitDepth` is the only place the width is stated, which is
    /// what a decoder has to trust here and cannot in an ISO BMFF fourcc.
    #[test]
    fn a_24_bit_pcm_track_decodes_block_by_block() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-int.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a PCM track");
        assert_eq!(stream.codec(), "A_PCM/INT/LIT");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        assert_eq!(stream.bits_per_sample(), 24);
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 11_025);
        // 2516582 of 2^23, the exact top of the sine as ffmpeg reads it.
        assert_eq!(peak, 2_516_582.0 / 8_388_608.0);
    }

    /// The same source and the same block layout in Matroska's float coding:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=0.25:s=44100' \
    ///   -vn -c:a pcm_f32le tests/fixtures/audio/pcm-float.mkv
    /// ```
    /// Float PCM is already the shape the pipeline carries, so this is the path
    /// where the decoder only has to read the width and hand the samples over.
    #[test]
    fn a_float_pcm_track_decodes_block_by_block() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-float.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a float PCM track");
        assert_eq!(stream.codec(), "A_PCM/FLOAT/IEEE");
        assert_eq!(stream.bits_per_sample(), 32);
        assert!(stream.extra_data().is_empty(), "PCM has no setup record");
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 11_025);
        assert!((0.2999..=0.3001).contains(&peak), "peak={peak}");
    }

    /// A listener with no picture takes the timeline total from the track, and
    /// Matroska states it one block at a time: the span of the track's own
    /// blocks plus one more interval. Block stamps sit on the cluster's grid, so
    /// the estimate lands within a block of the sound the fixture carries.
    #[test]
    fn the_blocks_state_the_track_length() {
        for (name, fixture, low, high) in [
            (
                "mp3",
                include_bytes!("../../../tests/fixtures/audio/mp3-stereo.mkv") as &[u8],
                2.02,
                2.05,
            ),
            (
                "flac",
                include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv") as &[u8],
                1.98,
                2.06,
            ),
            (
                "vorbis",
                include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm") as &[u8],
                1.99,
                2.06,
            ),
            (
                "pcm",
                include_bytes!("../../../tests/fixtures/audio/pcm-int.mkv") as &[u8],
                0.24,
                0.26,
            ),
        ] {
            let stream = WebmAudioReader::open(Cursor::new(fixture), Limits::default())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let at = stream
                .duration()
                .unwrap_or_else(|| panic!("{name} states no length"));
            assert!(
                (low..=high).contains(&at.as_secs_f64()),
                "{name}: {at:?} outside {low}..={high}"
            );
        }
    }

    #[test]
    fn only_the_stream_info_body_of_a_flac_setup_block_is_read() {
        let mut private = b"fLaC".to_vec();
        // Last-block flag set, type 0 (STREAMINFO), a 34-byte body.
        private.extend_from_slice(&[0x80, 0, 0, 34]);
        private.extend_from_slice(&[7u8; 34]);
        assert_eq!(flac_stream_info(&private), vec![7u8; 34]);
        // A block that is not STREAMINFO, or one that lies about its length, is
        // handed to the decoder as it stands.
        let comment = vec![0x04, 0, 0, 10];
        assert_eq!(flac_stream_info(&comment), comment);
        assert_eq!(
            flac_stream_info(&[0, 0, 0, 9, 1, 2]),
            vec![0, 0, 0, 9, 1, 2]
        );
    }

    fn bytes_per_frame(channels: u16) -> usize {
        usize::from(channels) * size_of::<f32>()
    }
}

impl<R: Read + Seek + Send> AudioStream for WebmAudioReader<R> {
    fn stereo_router(&self)->Result<Option<fvid_media::owned_stereo_router::StereoRouter>>{
        if self.codec_tag!="mp4a"&&self.codec_tag!="A_AAC"{return self.speaker_mask()?.map(|mask|fvid_media::owned_stereo_router::StereoRouter::new(self.channels(),mask).map_err(|e|invalid(&e))).transpose();}
        let asc=crate::codec::config::aac_specific_config(&self.extra_data)?;
        fvid_media::owned_aac::speaker_routing::stereo_router(asc,self.sample_rate(),self.channels(),self.in_band_ps).map(Some).map_err(|e|invalid(&e))
    }

    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        if self.codec_tag=="A_WAVPACK4" {
            return Ok(Box::new(crate::codec::wavpack_decoder::WavpackDecoder::for_matroska(
                self.extra_data(),self.sample_rate(),self.channels())?.with_speaker_mask(self.wavpack_mask)?));
        }
        if self.codec_tag == "A_AC3" {
            return Ok(Box::new(crate::codec::ac3_decoder::Ac3Decoder::for_container(self.extra_data(),self.sample_rate(),self.channels())?));
        }
        if self.codec_tag == "A_EAC3" {
            if self.eac3_stereo_output {
                if self.dolby_channels.unwrap_or(self.track().channels as u16)>6 {return Err(invalid("E-AC-3 extended dependent presentation stereo folding is not implemented"));}
                return Ok(Box::new(crate::codec::eac3_decoder::Eac3Decoder::for_container_stereo(self.extra_data(),self.sample_rate())?.with_programme(self.eac3_programme)?));
            }
            return Ok(Box::new(crate::codec::eac3_decoder::Eac3Decoder::for_container(self.extra_data(),self.sample_rate(),self.channels())?.with_programme(self.eac3_programme)?));
        }
        if self.in_band_ps {
            Ok(Box::new(
                crate::codec::aac_ps_playback::PsAacDecoder::new_with_in_band_ps(
                    &self.extra_data,
                    self.sample_rate(),
                    2,
                )?,
            ))
        } else {
            crate::codec::make_audio_decoder(
                self.codec(),
                self.extra_data(),
                self.sample_rate(),
                self.channels(),
                self.bits_per_sample(),
            )
        }
    }

    fn preroll_target(&self) -> Option<i64> {
        if self.codec_tag == "A_MPEG/L3" && !self.mpeg_checkpoint_safe { return None; }
        if self.codec_tag == "mp4a" && !self.aac_checkpoint_safe.get() { return None; }
        matches!(self.codec_tag.as_str(), "mp4a" | "A_VORBIS" | "A_MPEG/L3" | "A_MPEG/L1" | "A_MPEG/L2")
            .then(|| self.presentation_floor.unwrap_or(0))
    }
    fn resume_preroll(&mut self, pts: i64) -> bool {
        // These factories do not retain paired decoder/sample-clock snapshots.
        if matches!(self.codec_tag.as_str(), "A_MPEG/L1" | "A_MPEG/L2") {return false;}
        if self.preroll_target().is_none_or(|target| pts > target) {
            return false;
        }
        if self.codec_tag == "mp4a" {
            let Some((_, index, clock, outputs, metadata)) = self.aac_clock_checkpoints.iter().find(|(at,_,_,_,_)| *at == pts) else { return false; };
            self.packet_index = *index;
            *self.aac_clock.get_mut() = clock.clone();
            self.aac_output_packets.set(*outputs);
            self.aac_metadata_cursor.set(*metadata);
            return true;
        }
        if self.codec_tag == "A_MPEG/L3" {
            let Some((_, index, clock)) = self.mpeg_clock_checkpoints.iter()
                .find(|(at,_,_)|*at==pts) else {return false;};
            self.packet_index = *index;
            self.mpeg_clock = clock.clone();
            return true;
        }
        let delay = i128::from(self.index_delay_ns());
        let Some(index) = self
            .demuxer
            .packets
            .iter()
            .position(|p| p.track == self.track_number && i128::from(p.pts_ns) - delay == i128::from(pts))
        else {
            return false;
        };
        self.packet_index = index;
        true
    }

    fn set_eac3_stereo_output(&mut self, enabled: bool) -> Result<()> {
        if enabled && self.codec_tag=="A_EAC3" && self.dolby_channels.unwrap_or(self.track().channels as u16)>6 {
            return Err(invalid("E-AC-3 extended dependent presentation stereo folding is not implemented"));
        }
        self.eac3_stereo_output=enabled;
        Ok(())
    }
    fn speaker_mask(&self)->Result<Option<u32>>{
        if self.eac3_stereo_output && self.codec_tag=="A_EAC3" {return Ok(Some(3));}
        if matches!(self.codec_tag.as_str(),"A_AC3"|"A_EAC3") {return Ok(self.dolby_mask);}
        if self.codec_tag=="A_WAVPACK4" {return Ok(self.wavpack_mask);}
        if self.codec_tag=="A_ALAC" {return crate::codec::alac_decoder::AlacDecoder::configuration_mask(&self.extra_data,self.sample_rate(),self.channels()).map(Some);}
        if self.codec_tag!="A_FLAC" {return Ok(None);}
        let track=self.demuxer.tracks.iter().find(|t|t.number==self.track_number).ok_or_else(||invalid("selected FLAC track is absent"))?;
        let mask=fvid_media::owned_flac_layout::channel_mask(&track.codec_private).map_err(|e|invalid(&e.to_string()))?;
        let mask=mask.unwrap_or_else(||fvid_media::owned_flac_layout::default_channel_mask(self.channels()));
        if mask.count_ones()>u32::from(self.channels()) {return Err(invalid("FLAC channel mask exceeds stream channel count"));}
        Ok(Some(mask))
    }
    fn codec(&self) -> &str {
        &self.codec_tag
    }

    fn timescale(&self) -> u32 {
        TIMESCALE_NS
    }

    fn sample_rate(&self) -> u32 {
        if self.codec_tag == "A_OPUS" {
            48000
        } else {
            self.dolby_rate.unwrap_or(self.track().sample_rate as u32)
        }
    }

    fn channels(&self) -> u16 {
        if self.in_band_ps || (self.eac3_stereo_output && self.codec_tag=="A_EAC3") {
            2
        } else {
            self.dolby_channels.unwrap_or(self.track().channels as u16)
        }
    }

    /// Presentation endpoint from known indexed timing where available.
    /// Opus projects its audible windows independently of decode progress;
    /// streams without usable timing retain the legacy length estimates.
    fn duration(&self) -> Option<std::time::Duration> {
        if self.codec_tag.starts_with("A_PCM/")&&self.demuxer.fully_indexed(){return self.pcm_duration();}
        if self.codec_tag=="A_ALAC" && self.alac_duration_complete.get(){return self.alac_end_ns.get().map(std::time::Duration::from_nanos);}
        if self.codec_tag=="A_FLAC" && self.flac_duration_complete {
            return self.flac_end_ns.map(std::time::Duration::from_nanos);
        }
        if self.codec_tag == "mp4a" {
            let complete = self.aac_duration_complete.get()
                || (self.demuxer.fully_indexed()
                    && self.aac_output_packets.get() > 0
                    && self.aac_output_packets.get() == self.demuxer.packets.iter().filter(|p| p.track == self.track_number).count());
            if complete {
                self.aac_duration_complete.set(true);
                return self.aac_end_ns.get().map(std::time::Duration::from_nanos);
            }
        }
        if matches!(self.codec_tag.as_str(), "A_AC3" | "A_EAC3") {
            let endpoint = if self.ac3_duration_complete { self.ac3_end_ns.get() } else { None };
            return endpoint.or(self.demuxer.duration_ns).map(std::time::Duration::from_nanos);
        }
        if matches!(self.codec_tag.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3") && self.demuxer.fully_indexed() {
            let rate=self.sample_rate(); if rate==0 {return None;}
            let frames=match self.codec_tag.as_str(){"A_MPEG/L1"=>384,"A_MPEG/L2"=>1152,_=>if rate>=32000{1152}else{576}};
            let mut clock=MpegClock::default();let mut end=0u64;let mut seen=false;
            for (index,packet) in self.demuxer.packets.iter().enumerate().filter(|(_,p)|p.track==self.track_number) {
                clock.stamp_aac(self.indexed_timestamp(packet.pts_ns)?,frames,rate,self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(index),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0)).ok()?;
                let padding=discard_padding_samples(packet.discard_padding_ns,rate);
                if padding>frames as u128 || discard_padding_exceeds_packet(packet.discard_padding_ns,rate,frames) {return None;}
                let first=clock.skip_delayed(0,rate,frames,self.index_delay_ns()).max(if packet.discard_padding_ns<0 {padding as usize}else{0});
                let last=frames-if packet.discard_padding_ns>0 {padding as usize}else{0};
                if first<last {end=end.max(clock.presented(last,rate,self.index_delay_ns())?);seen=true;}
            }
            return seen.then(||std::time::Duration::from_nanos(end));
        }
        if self.codec_tag == "A_OPUS" && self.demuxer.fully_indexed() {
            if let Some(end) = *self.opus_indexed_end.get_or_init(|| self.indexed_opus_end_ns()) {
                return Some(std::time::Duration::from_nanos(end));
            }
            if self.opus_duration_complete && let Some(end) = self.opus_end_ns {
                return Some(std::time::Duration::from_nanos(end));
            }
        }
        if self.codec_tag == "A_VORBIS" {
            // Index completion can precede presentation. A partially rebuilt
            // overlap clock is not the track duration; retain the endpoint
            // established at EOF, or use the declared duration until then.
            return self.vorbis_end_ns.or(self.demuxer.duration_ns)
                .map(std::time::Duration::from_nanos);
        }
        // Blocks are indexed a cluster at a time, so an index that has not met
        // the file's tail has no last block to measure against. What the muxer
        // stated for the whole item is the only answer such an index can give,
        // and the only one a file that states it needs no walk to reach.
        if !self.demuxer.fully_indexed() {
            return self
                .demuxer
                .duration_ns
                .map(std::time::Duration::from_nanos);
        }
        if !matches!(self.codec_tag.as_str(), "A_OPUS" | "A_VORBIS" | "mp4a") {
            // Explicit block/default durations let the endpoint remain stable
            // across decode, rewind and seek, including fully discarded tails.
            let rate = u128::from(self.sample_rate());
            if rate == 0 { return None; }
            let default_duration = self.track().default_duration_ns;
            let delay = i128::from(self.track().codec_delay_ns);
            let mut known = true;
            let mut seen = false;
            let mut endpoint = 0i128;
            for packet in self.demuxer.packets.iter().filter(|p| p.track == self.track_number) {
                seen = true;
                let Some(duration) = packet.duration_ns.filter(|&d|d != 0)
                    .or_else(|| (default_duration != 0).then_some(default_duration)) else {
                    known = false; break;
                };
                let samples = discard_padding_samples(packet.discard_padding_ns, self.sample_rate());
                let trim = i128::try_from(samples * 1_000_000_000 / rate).ok()?;
                if trim > i128::from(duration) { return None; }
                let source = i128::from(packet.pts_ns) - delay;
                let start = source + if packet.discard_padding_ns < 0 { trim } else { 0 };
                let end = source + i128::from(duration)
                    - if packet.discard_padding_ns > 0 { trim } else { 0 };
                if end > start.max(0) { endpoint = endpoint.max(end); }
            }
            if known && seen {
                return Some(std::time::Duration::from_nanos(u64::try_from(endpoint).ok()?));
            }
        }
        let mut stamps = self
            .demuxer
            .packets
            .iter()
            .filter(|p| p.track == self.track_number)
            .map(|p| p.pts_ns);
        let first = stamps.next()?;
        let mut last = first;
        let mut gap = None;
        for pts in stamps {
            if pts > last {
                gap = Some(pts - last);
                last = pts;
            }
        }
        let tail = gap.or_else(|| {
            i64::try_from(self.track().default_duration_ns)
                .ok()
                .filter(|declared| *declared > 0)
        })?;
        let nanos = i128::from(last) - i128::from(first) + i128::from(tail);
        Some(std::time::Duration::from_nanos(u64::try_from(nanos).ok()?))
    }

    fn bits_per_sample(&self) -> u16 {
        // A width too large for the type cannot be a real sample depth, so it
        // reaches the decoder as one it will refuse rather than as the 0 that
        // means "undeclared" and stands for 16-bit.
        u16::try_from(self.track().bit_depth).unwrap_or(u16::MAX)
    }

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    fn audio_track_index(&self) -> usize {
        Self::supported(&self.demuxer)
            .iter()
            .position(|&number| number == self.track_number)
            .unwrap_or(0)
    }
    fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
        WebmAudioReader::supported(&self.demuxer)
            .into_iter()
            .map(|number| {
                let track = self
                    .demuxer
                    .tracks
                    .iter()
                    .find(|t| t.number == number)
                    .expect("supported lists existing tracks");
                crate::audio::AudioTrack {
                    sample_rate: if number==self.track_number{self.sample_rate()}else if track.codec == "A_OPUS" {
                        48000
                    } else {
                        track.sample_rate as u32
                    },
                    channels: if number==self.track_number {self.channels()}else{track.channels as u16},
                    name: track.name.clone(),
                    language: track.language.clone(),
                }
            })
            .collect()
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(mut p) = WebmAudioReader::read_packet(self)? else {
            if matches!(self.codec_tag.as_str(), "A_AC3" | "A_EAC3") { self.ac3_duration_complete = true; }
            if self.codec_tag == "A_FLAC" { self.flac_duration_complete = true; }
            if self.codec_tag=="A_ALAC" && self.demuxer.fully_indexed() && self.alac_outputs.get()==self.demuxer.packets.iter().filter(|p|p.track==self.track_number).count(){self.alac_duration_complete.set(true);}
            if self.codec_tag == "mp4a" && self.aac_output_packets.get() > 0
                && self.aac_output_packets.get() == self.demuxer.packets.iter().filter(|p| p.track == self.track_number).count() {
                self.aac_duration_complete.set(true);
            }
            if self.codec_tag == "A_OPUS" && self.opus_clock.packets == self.demuxer.packets.iter().filter(|p| p.track == self.track_number).count() {
                self.opus_duration_complete = true;
            }
            if let Some(clock) = &self.vorbis {
                self.vorbis_end_ns = clock.end_ns;
            }
            return Ok(None);
        };
        let duration = if self.codec_tag=="A_FLAC" {
            self.stamp_flac_packet(&mut p)?
        } else if self.codec_tag.starts_with("A_PCM/") {
            let(stride,rate)=self.pcm_geometry()?;
            if !p.data.len().is_multiple_of(stride){return Err(invalid("PCM packet ends in an incomplete channel frame"));}
            let index=self.packet_index-1;let grid=self.demuxer.packet_timestamp_grid(index);
            let(pts,duration)=self.pcm_clock.stamp_aac(p.pts_ns,p.data.len()/stride,rate,
                self.demuxer.timestamp_scale_ns(),grid,self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0))?;
            p.pts_ns=pts;duration
        } else if self.codec_tag == "mp4a" {
            if self.aac_checkpoint_safe.get()
                && !self.aac_clock_checkpoints.iter().any(|(at,_,_,_,_)| at / 1_000_000_000 == p.pts_ns / 1_000_000_000) {
                if self.aac_clock_checkpoints.len() == 32 { self.aac_clock_checkpoints.remove(0); }
                self.aac_clock_checkpoints.push((p.pts_ns, self.packet_index - 1,
                    self.aac_clock.borrow().clone(), self.aac_output_packets.get(), self.aac_metadata_cursor.get()));
            }
            0
        } else if self.codec_tag == "A_VORBIS" {
            let rate = self.sample_rate();
            let tick = self.demuxer.timestamp_scale_ns();
            let clock = self
                .vorbis
                .as_mut()
                .ok_or_else(|| invalid("missing Vorbis clock"))?;
            let (pts, duration) = clock.stamp(&p.data, p.pts_ns, rate, tick)?;
            p.pts_ns = pts;
            // Restore the same quantized sample count used by PCM trimming.
            let padding_samples = vorbis_padding_samples(self.discard_padding_ns.max(0), rate);
            let samples = u128::from(clock.samples).checked_sub(padding_samples)
                .ok_or_else(|| invalid("Vorbis discard padding exceeds timeline"))?;
            let end = i128::from(clock.origin.unwrap())
                + i128::try_from(samples * 1_000_000_000 / u128::from(rate))
                    .map_err(|_| invalid("Vorbis end overflow"))?;
            clock.end_ns = Some(u64::try_from(end.max(0))
                .map_err(|_| invalid("Vorbis end overflow"))?);
            duration
        } else if matches!(self.codec_tag.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3") {
            let (samples, rate) = if self.codec_tag == "A_MPEG/L1" {
                (384, fvid_codecs::mp1::Header::parse(&p.data)?.sample_rate)
            } else if self.codec_tag == "A_MPEG/L2" {
                (1152, fvid_codecs::mp2::Header::parse(&p.data)?.sample_rate)
            } else {
                let header = fvid_codecs::mp3_frame::Header::parse(&p.data)?;
                (header.samples(), header.sample_rate)
            };
            let before = self.mpeg_clock.clone();
            if before.origin.is_some_and(|origin| i128::from(p.pts_ns)
                + i128::from(self.demuxer.timestamp_scale_ns())
                < i128::from(origin) + i128::from(before.samples)*1_000_000_000/i128::from(rate)) {
                // A PTS-keyed checkpoint could skip an earlier occurrence of
                // the requested time. Retain full replay for this timeline.
                self.mpeg_checkpoint_safe=false;
                self.mpeg_clock_checkpoints.clear();
            }
            let (pts,duration)=self.mpeg_clock.stamp_aac(p.pts_ns,samples,
                rate,self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(self.packet_index-1),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0))?;
            p.pts_ns=pts;
            if self.codec_tag == "A_MPEG/L3" && self.mpeg_checkpoint_safe && !self.mpeg_clock_checkpoints.iter().any(|(at,_,_)|at/1_000_000_000==pts/1_000_000_000) {
                if self.mpeg_clock_checkpoints.len()==32 {self.mpeg_clock_checkpoints.remove(0);}
                self.mpeg_clock_checkpoints.push((pts,self.packet_index-1,before));
            }
            duration
        } else if self.codec_tag == "A_AC3" {
            let mut offset=0usize;let mut frames=0usize;
            while offset<p.data.len() {
                let header=crate::codec::ac3_decoder::syncframe(&p.data[offset..]).ok_or_else(||invalid("AC-3 packet holds no syncframe"))?;
                frames=frames.checked_add(1536).ok_or_else(||invalid("AC-3 packet sample count overflow"))?;
                offset=offset.checked_add(header.frame_bytes).ok_or_else(||invalid("AC-3 frame size overflow"))?;
            }
            let (pts,duration)=self.ac3_clock.stamp(p.pts_ns,frames,self.sample_rate(),self.demuxer.timestamp_scale_ns())?;
            p.pts_ns=pts;duration
        } else if self.codec_tag == "A_EAC3" {
            let mut offset=0usize;let mut frames=0usize;
            while offset<p.data.len() {
                if let Some(size)=crate::codec::eac3_decoder::skipped_programme_bytes(&p.data[offset..],self.eac3_programme)? {
                    offset+=size;
                    continue;
                }
                let header=crate::codec::eac3_decoder::programme_frame(&p.data[offset..],self.eac3_programme).ok_or_else(||invalid("E-AC-3 packet holds no syncframe"))?;
                frames=frames.checked_add(header.blocks * 256).ok_or_else(||invalid("E-AC-3 packet sample count overflow"))?;
                offset=offset.checked_add(header.frame_bytes).ok_or_else(||invalid("E-AC-3 frame size overflow"))?;
            }
            let (pts,duration)=if frames==0 {(p.pts_ns,0)}else{self.ac3_clock.stamp(p.pts_ns,frames,self.sample_rate(),self.demuxer.timestamp_scale_ns())?};
            if frames!=0 {
                self.eac3_windows.get_mut().push_back((pts,self.ac3_clock.clone(),self.discard_padding_ns));
            }
            p.pts_ns=pts;duration
        } else if self.codec_tag == "A_OPUS" {
            let duration =
                crate::codec::opus_decoder::duration_ns(&self.track().codec_private, &p.data)?;
            let preskip = u64::from(crate::codec::opus_decoder::header(&self.track().codec_private)?.pre_skip);
            let delay_ns=if self.track().codec_delay_ns!=0{self.track().codec_delay_ns}else{preskip*1_000_000_000/48000};
            let timing = self.opus_clock.stamp(p.pts_ns, duration, preskip, self.demuxer.timestamp_scale_ns(),self.demuxer.packet_timestamp_grid(self.packet_index-1),self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0),delay_ns)?;
            p.pts_ns = timing.pts; self.opus_packet_preskip = timing.preskip;
            let endpoint = self.opus_end_ns.get_or_insert(0);
            if let Some(end) = timing.audible_end(self.discard_padding_ns)? { *endpoint = (*endpoint).max(end); }
            duration as i64
        } else {
            0
        };
        Ok(Some(EncodedPacket {
            data: p.data,
            pts: p.pts_ns,
            duration,
        }))
    }

    fn rewind(&mut self) {
        WebmAudioReader::rewind(self);
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        if self.codec_tag=="A_WAVPACK4" {
            if self.wavpack_dsd {self.rewind();self.presentation_floor=Some(pts.max(0));return pts.max(0);}
            // Every WavPack block carries its predictor/entropy state. Seek
            // directly, retaining a presentation floor for a partial block.
            self.presentation_floor=Some(pts.max(0));
            return WebmAudioReader::seek(self,pts.max(0));
        }
        if self.codec_tag=="A_ALAC" {
            self.rewind();let target=pts.max(0);self.presentation_floor=Some(target);
            if let Err(error)=self.seek_alac(target){self.pcm_seek_error=Some(error.to_string());}
            return target;
        }
        if self.codec_tag.starts_with("A_PCM/") {
            self.rewind();let target=pts.max(0);self.presentation_floor=Some(target);
            if let Err(error)=self.seek_pcm(target){self.pcm_seek_error=Some(error.to_string());}
            return target;
        }
        if matches!(self.codec_tag.as_str(), "A_OPUS" | "A_VORBIS" | "A_MPEG/L3" | "A_MPEG/L1" | "A_MPEG/L2" | "A_AC3" | "A_EAC3" | "A_FLAC" | "A_ALAC" | "mp4a") {
            // Rebuild decoder history from the stream's beginning so even a
            // short or mode-switching track seeks to the same PCM as playback.
            self.rewind();
            self.presentation_floor = Some(pts.max(0));
            return pts.max(0);
        }
        WebmAudioReader::seek(self, pts)
    }

    fn present_decoded(
        &self,
        mut packet: crate::audio::AudioPacket,
        source_pts: i64,
    ) -> Result<Option<crate::audio::AudioPacket>> {
        if self.codec_tag == "A_VORBIS" {
            let stride = usize::from(self.channels()) * 4;
            let rate = u128::from(self.sample_rate());
            if stride == 0 || !packet.data.len().is_multiple_of(stride) {
                return Err(invalid("invalid Vorbis PCM stride"));
            }
            let frames = packet.data.len() / stride;
            let padding = usize::try_from(
                vorbis_padding_samples(self.discard_padding_ns, self.sample_rate()),
            )
            .map_err(|_| invalid("Vorbis discard padding overflow"))?;
            if padding > frames || discard_padding_exceeds_packet(self.discard_padding_ns, self.sample_rate(), frames) {
                return Err(invalid("Vorbis discard padding exceeds decoded packet"));
            }
            let target = self.presentation_floor.unwrap_or(0).max(0);
            let skip = if target == 0 {
                zero_origin_preroll_samples(source_pts, self.sample_rate(), frames)
            } else {
                (((i128::from(target) - i128::from(source_pts)).max(0) as u128 * rate)
                    .div_ceil(1_000_000_000)).min(frames as u128) as usize
            };
            let first = skip.max(if self.discard_padding_ns < 0 {
                padding
            } else {
                0
            });
            let last = frames
                - if self.discard_padding_ns > 0 {
                    padding
                } else {
                    0
                };
            if first >= last {
                return Ok(None);
            }
            packet.data.copy_within(first * stride..last * stride, 0);
            packet.data.truncate((last - first) * stride);
            packet.pts = presentation_after_preroll(source_pts, first, self.sample_rate())
                .ok_or_else(|| invalid("negative Vorbis presentation time"))?;
            packet.timebase_num = 1;
            packet.timebase_den = TIMESCALE_NS;
        }
        if self.codec_tag == "A_OPUS" {
            let stride = usize::from(self.channels()) * 4;
            if stride == 0 || !packet.data.len().is_multiple_of(stride) {
                return Err(invalid("invalid Opus PCM stride"));
            }
            let frames = packet.data.len() / stride;
            let padding = discard_padding_samples(self.discard_padding_ns, 48000);
            let padding =
                usize::try_from(padding).map_err(|_| invalid("Opus discard padding overflow"))?;
            if padding > frames || discard_padding_exceeds_packet(self.discard_padding_ns, 48000, frames) {
                return Err(invalid("Opus discard padding exceeds decoded packet"));
            }
            let floor = self.presentation_floor.unwrap_or(0).max(0);
            let skip=self.opus_clock.timeline.exact_skip(floor,48000,frames,self.opus_clock.delay_ns).ok_or_else(||invalid("invalid Opus sample clock"))?;
            let first = skip
                .max(self.opus_packet_preskip)
                .max(if self.discard_padding_ns < 0 {
                    padding
                } else {
                    0
                });
            let last = frames
                - if self.discard_padding_ns > 0 {
                    padding
                } else {
                    0
                };
            if first >= last {
                return Ok(None);
            }
            packet.data.copy_within(first * stride..last * stride, 0);
            packet.data.truncate((last - first) * stride);
            packet.pts = self.opus_clock.timeline.exact_point(first,48000,self.opus_clock.delay_ns)
                .ok_or_else(|| invalid("negative Opus presentation time"))?;
            packet.timebase_num = 1;
            packet.timebase_den = TIMESCALE_NS;
        }
        if !matches!(self.codec_tag.as_str(), "A_OPUS" | "A_VORBIS" | "mp4a") {
            let stride = usize::from(self.channels()) * 4;
            let rate = self.sample_rate();
            if stride == 0 || rate == 0 || !packet.data.len().is_multiple_of(stride) {
                return Err(invalid("invalid Matroska PCM geometry"));
            }
            let frames = packet.data.len() / stride;
            let context=if self.codec_tag=="A_EAC3" {
                let mut windows=self.eac3_windows.borrow_mut();
                if windows.front().is_some_and(|w|w.0!=source_pts) {
                    return Err(invalid("E-AC-3 output source metadata identity mismatch"));
                }
                windows.pop_front()
            }else{None};
            let dolby_clock=context.as_ref().map_or(&self.ac3_clock,|w|&w.1);
            let padding_ns=context.as_ref().map_or(self.discard_padding_ns,|w|w.2);
            let padding = discard_padding_samples(padding_ns, rate);
            if padding > frames as u128 || discard_padding_exceeds_packet(padding_ns, rate, frames) {
                return Err(invalid("Matroska discard padding exceeds decoded packet"));
            }
            let padding = padding as usize;
            let source_pts=if self.codec_tag=="A_ALAC" {self.stamp_alac_geometry(source_pts,frames)?}else{source_pts};
            // Public presentation calls can provide a standalone decoded PCM
            // packet before any indexed packet has initialized its clock.
            // Keep those signed timestamp/geometry checks, while indexed
            // playback continues to use its accumulated exact sample history.
            let direct_pcm_clock=MpegClock {origin:Some(source_pts),..Default::default()};
            let pcm_clock=if self.pcm_clock.origin.is_some(){&self.pcm_clock}else{&direct_pcm_clock};
            // Audio decoders preserve the supplied integer timestamp, but
            // label it with their sample clocks. Matroska timestamps are ns.
            // Trim negative presentation samples before changing that clock.
            let skip = if self.codec_tag.starts_with("A_PCM/") {
                pcm_clock.exact_skip(self.presentation_floor.unwrap_or(0).max(0),rate,frames,self.index_delay_ns())
                    .ok_or_else(||invalid("invalid PCM sample clock"))?
            } else if self.codec_tag=="A_ALAC" {
                self.alac_clock.borrow().exact_skip(self.presentation_floor.unwrap_or(0).max(0),rate,frames,self.index_delay_ns()).ok_or_else(||invalid("invalid ALAC sample clock"))?
            } else if self.codec_tag=="A_FLAC" {
                self.flac_clock.exact_skip(self.presentation_floor.unwrap_or(0).max(0),rate,frames,self.index_delay_ns()).ok_or_else(||invalid("invalid FLAC sample clock"))?
            } else if matches!(self.codec_tag.as_str(), "A_AC3" | "A_EAC3") {
                dolby_clock.skip_delayed(self.presentation_floor.unwrap_or(0).max(0),rate,frames,self.index_delay_ns())
            } else if matches!(self.codec_tag.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3") {
                self.mpeg_clock.skip_delayed(self.presentation_floor.unwrap_or(0).max(0),rate,frames,self.index_delay_ns())
            } else if self.codec_tag=="A_WAVPACK4" {
                (((i128::from(self.presentation_floor.unwrap_or(0).max(0))-i128::from(source_pts))
                    .max(0) as u128)*u128::from(rate)).div_ceil(1_000_000_000).min(frames as u128) as usize
            } else { zero_origin_preroll_samples(source_pts, rate, frames) };
            let first = skip.max(if padding_ns < 0 { padding } else { 0 });
            let last = frames - if padding_ns > 0 { padding } else { 0 };
            if matches!(self.codec_tag.as_str(), "A_AC3" | "A_EAC3") && dolby_clock.skip_delayed(0,rate,frames,self.index_delay_ns())<last {
                let end=dolby_clock.presented(last,rate,self.index_delay_ns()).ok_or_else(||invalid("negative AC-3 presentation end"))?;
                self.ac3_end_ns.set(Some(self.ac3_end_ns.get().unwrap_or(0).max(end)));
            }
            if first >= last { return Ok(None); }
            if first != 0 { packet.data.copy_within(first * stride..last * stride, 0); }
            packet.data.truncate((last - first) * stride);
            packet.pts = if self.codec_tag.starts_with("A_PCM/"){pcm_clock.exact_point(first,rate,self.index_delay_ns())}
                else if self.codec_tag=="A_ALAC" {self.alac_clock.borrow().exact_point(first,rate,self.index_delay_ns())}
                else if self.codec_tag=="A_FLAC" {self.flac_clock.exact_point(first,rate,self.index_delay_ns())}
                else if matches!(self.codec_tag.as_str(), "A_AC3" | "A_EAC3") {dolby_clock.presented(first,rate,self.index_delay_ns())}
                else if matches!(self.codec_tag.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3") {self.mpeg_clock.presented(first,rate,self.index_delay_ns())}
                else {presentation_after_preroll(source_pts, first, rate)}
                .ok_or_else(|| invalid("negative PCM presentation time"))?;
            packet.timebase_num = 1;
            packet.timebase_den = TIMESCALE_NS;
        }
        if self.codec_tag == "mp4a" {
            let stride = usize::from(self.channels()) * 4;
            let rate = self.sample_rate();
            if stride == 0 || rate == 0 || !packet.data.len().is_multiple_of(stride) {
                return Err(invalid("invalid AAC PCM geometry"));
            }
            let frames = packet.data.len() / stride;
            // Metadata belongs to the decoded source AU, including PS output
            // delayed until a following packet or EOF. A cursor avoids a PTS
            // lookup that would confuse duplicate timestamps.
            let (metadata_index, metadata) = self.demuxer.packets.iter().enumerate()
                .skip(self.aac_metadata_cursor.get()).take(self.packet_index.saturating_sub(self.aac_metadata_cursor.get()))
                .find(|(_, p)| p.track == self.track_number)
                .ok_or_else(|| invalid("AAC output has no source packet metadata"))?;
            if self.indexed_timestamp(metadata.pts_ns) != Some(source_pts) {
                return Err(invalid("AAC output source metadata identity mismatch"));
            }
            let padding = discard_padding_samples(metadata.discard_padding_ns, rate);
            if padding > frames as u128 || discard_padding_exceeds_packet(metadata.discard_padding_ns, rate, frames) {
                return Err(invalid("Matroska discard padding exceeds AAC packet"));
            }
            let head = if metadata.discard_padding_ns < 0 { padding as usize } else { 0 };
            let last = frames - if metadata.discard_padding_ns > 0 { padding as usize } else { 0 };
            self.aac_metadata_cursor.set(metadata_index + 1);
            let floor = self.presentation_floor.unwrap_or(0).max(0);
            // Decode negative/source preroll to retain codec history, but only
            // present samples at or after the floor. Delayed PS output uses
            // its original source PTS, never the lookahead packet's timestamp.
            // Decode order supplies the exact sample count, including delayed PS
            // output. Snap only within the container quantization tick; a real
            // discontinuity starts a new sample epoch instead of being hidden.
            let factor=self.demuxer.track_timestamp_scale(self.track_number).unwrap_or(1.0);
            let tick=self.demuxer.timestamp_scale_ns();
            let precision=fvid_media::owned_matroska_time::duration_ns(1,tick,factor)
                .map_or(u64::MAX,|v|v.saturating_add(1)).max(tick);
            let grid=self.demuxer.packet_timestamp_grid(metadata_index);
            let mut clock = self.aac_clock.borrow_mut();
            if clock.origin.is_some_and(|origin| i128::from(source_pts)
                + i128::from(precision)
                < i128::from(origin) + i128::from(clock.samples) * 1_000_000_000 / i128::from(rate)) {
                // A PTS alone cannot identify which occurrence to restore.
                self.aac_checkpoint_safe.set(false);
            }
            let (source_pts, _) = clock.stamp_aac(source_pts,frames,rate,tick,grid,factor)?;
            let origin = i64::try_from(i128::from(clock.origin.unwrap()) + i128::from(self.index_delay_ns()))
                .map_err(|_| invalid("AAC sample clock origin overflow"))?;
            let (_, end) = aac_sample_window(source_pts, i64::MAX, rate, last, origin, self.index_delay_ns());
            if head < last && let Ok(end) = u64::try_from(end) {
                self.aac_end_ns.set(Some(self.aac_end_ns.get().unwrap_or(0).max(end)));
            }
            self.aac_output_packets.set(self.aac_output_packets.get().checked_add(1)
                .ok_or_else(|| invalid("AAC output packet count overflow"))?);
            let (skip,_) = aac_sample_window(source_pts,floor,rate,frames,origin,self.index_delay_ns());
            let first = skip.max(head);
            if first >= last { return Ok(None); }
            let (_,at) = aac_sample_window(source_pts,i64::MAX,rate,first,origin,self.index_delay_ns());
            packet.data.copy_within(first * stride..last * stride, 0);
            packet.data.truncate((last - first) * stride);
            packet.pts =
                u64::try_from(at).map_err(|_| invalid("negative AAC presentation time"))?;
            // AAC decoder timestamps use a sample clock; Matroska supplies ns.
            packet.timebase_num = 1;
            packet.timebase_den = TIMESCALE_NS;
        }
        Ok(Some(packet))
    }
}
#[cfg(test)]
mod aac_sample_window_regressions {
    use super::aac_sample_window;
    #[test]
    fn nanosecond_packet_rounding_never_discards_an_extra_whole_sample() {
        assert_eq!(
            aac_sample_window(42_666_666, 50_000_000, 48000, 1024, 0, 0),
            (352, 50_000_000)
        );
        assert_eq!(
            aac_sample_window(42_666_666, 50_000_001, 48000, 1024, 0, 0),
            (353, 50_020_833)
        );
        assert_eq!(
            aac_sample_window(42_666_666, 50_020_833, 48000, 1024, 0, 0),
            (353, 50_020_833)
        );
        assert_eq!(
            aac_sample_window(-354_167, 0, 48000, 1024, 0, 354_167),
            (17, 0)
        );
        assert_eq!(
            aac_sample_window(42_312_499, 0, 48000, 1024, 0, 354_167),
            (0, 42_312_500)
        );
        assert_eq!(
            aac_sample_window(43_000_000, 50_000_000, 44100, 1024, 0, 0),
            (309, 50_006_802)
        );
    }
}
