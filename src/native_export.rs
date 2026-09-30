//! Native planar Y4M export. No foreign decoder, encoder or muxer is used.
use crate::{
    Result, invalid,
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Export a constant-rate planar video. Variable timing/geometry is rejected
/// rather than silently retimed. The destination is created only on success;
/// an existing destination (including a symlink) is never replaced.
pub fn export_y4m(source: &Path, destination: &Path) -> Result<u64> {
    export_y4m_interval(source, destination, None)
}

/// Export frames with presentation starts in [from, to), decoding reference
/// pre-roll through the owned codec. The resulting Y4M starts at time zero.
pub fn export_y4m_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<u64> {
    export_y4m_transformed(
        source,
        destination,
        interval,
        &crate::native_geometry::VideoGeometry::default(),
    )
}

/// Save owned geometry transformations after container display orientation.
/// Scaling retains the pre-scale display aspect by adjusting pixel aspect.
pub fn export_y4m_transformed(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<u64> {
    Ok(export_y4m_sources(&[source.to_owned()],destination,interval,geometry,false,None,None)?.packets)
}

/// Decode compatible video segments in order into one constant-rate Y4M stream.
/// Each source gets a fresh decoder; only raw presentation frames are appended.
pub fn concat_y4m(sources: &[PathBuf], destination: &Path, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    if !(2..=256).contains(&sources.len()) {return Err(invalid("concat requires 2..=256 inputs"));}
    if destination.extension().and_then(|s|s.to_str())!=Some("y4m") {return Err(invalid("native video concat output requires .y4m"));}
    for source in sources {
        if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
        validate_video_selection(source,selected)?;
    }
    export_y4m_sources(sources,destination,None,&Default::default(),false,cancel,progress)
}

pub(crate) fn validate_video_selection(source: &Path, selected: Option<usize>) -> Result<()> {
    use std::io::Read;
    let mut file=File::open(source)?;
    let mut prefix=[0;9];file.read_exact(&mut prefix)?;
    if &prefix==b"YUV4MPEG2" {
        if selected.is_some_and(|i|i!=0) {return Err(invalid("Y4M has only stream 0"));}
        return Ok(());
    }
    let info=crate::native_probe::probe(source).map_err(|e|invalid(&e))?;
    let videos:Vec<_>=info.streams.iter().filter(|s|s.media_type=="video").collect();
    if videos.len()!=1 {return Err(invalid("native video export requires exactly one video track per input"));}
    if selected.is_some_and(|i|i!=videos[0].index) || (selected.is_none() && info.streams.len()!=1) {
        return Err(invalid("video-only export requires explicit selection when other tracks exist"));
    }
    Ok(())
}

fn export_y4m_sources(sources: &[PathBuf], destination: &Path,
    interval: Option<(std::time::Duration,std::time::Duration)>, geometry: &crate::native_geometry::VideoGeometry,
    relative_interval: bool, cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut control=crate::native_media::DecodeProgress::new(cancel,progress)?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("export interval requires from < to"));
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, file) = (0..100)
        .find_map(|_| {
            let path = directory.join(format!(
                ".fvid-y4m-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((Temporary(path), file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve Y4M output"))??;
    let mut output = BufWriter::new(file);
    let mut output_rate: Option<(u128,u128)> = None;
    let mut layout = None;
    let mut count = 0;
    let mut payload_bytes=0u64;
    for source in sources {
    control.check()?;
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let segment_start=count;
    let mut origin=None;
    let mut previous: Option<(u128, u128, u128)> = None;
    while let Some(frame) = reader.read_frame_raw()? {
        control.check()?;
        let (start, end, scale) = reader
            .frame_interval()
            .ok_or_else(|| invalid("missing frame timing"))?;
        let first=*origin.get_or_insert(start);
        if let Some((from, to)) = interval {
            let relative_start=if relative_interval {start.checked_sub(first).ok_or_else(||invalid("frame precedes segment origin"))?} else {start};
            let stamp = product(relative_start, 1_000_000_000)?;
            if stamp >= product(to.as_nanos(), u128::from(scale))? {
                break;
            }
            if stamp < product(from.as_nanos(), u128::from(scale))? {
                continue;
            }
        }
        let duration = end
            .checked_sub(start)
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("invalid frame duration"))?;
        if let Some((old_end, old_duration, old_scale)) = previous {
            if product(start, old_scale)? != product(old_end, u128::from(scale))?
                || product(duration, old_scale)? != product(old_duration, u128::from(scale))?
            {
                return Err(invalid(
                    "Y4M export requires constant contiguous frame timing",
                ));
            }
        }
        control.check()?;
        if let Some((first_duration,first_scale))=output_rate {
            if product(duration,first_scale)?!=product(first_duration,u128::from(scale))? {
                return Err(invalid("Y4M concat requires identical frame rates"));
            }
        } else {output_rate=Some((duration,u128::from(scale)));}
        previous = Some((end, duration, u128::from(scale)));
        let [width, height] = reader.dimensions();
        let rotation = reader.rotation();
        let (mut chroma, depth) = match &frame {
            RawFrame::Planar(p) => (
                match p.frame.subsampling {
                    Some([2, 2]) => "420",
                    Some([2, 1]) => "422",
                    Some([1, 1]) => "444",
                    _ => return Err(invalid("unsupported Y4M chroma layout")),
                },
                p.depth,
            ),
            RawFrame::Avc { picture, .. }
                if width % 2 == 0
                    && height % 2 == 0
                    && picture.cb.len()
                        == picture.coded_width.div_ceil(2) * picture.coded_height.div_ceil(2)
                    && picture.cr.len() == picture.cb.len() =>
            {
                ("420", picture.bit_depth)
            }
            RawFrame::Planar8(p) if p.chroma_width == p.width && p.chroma_height == p.height => {
                ("444", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height =>
            {
                ("422", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height.div_ceil(2) =>
            {
                ("420", 8)
            }
            _ => return Err(invalid("Y4M export requires supported planar frames")),
        };
        let full = match &frame {
            RawFrame::Avc { colour, .. } => colour.full,
            RawFrame::Planar8(p) => p.colour.full,
            RawFrame::Planar(p) => p.colour.full,
            _ => false,
        };
        let aspect = transformed_aspect(reader.pixel_aspect(), width, height, geometry)?;
        let transformed = if geometry.is_identity() && rotation == 0 {
            None
        } else {
            Some(geometry.apply_display(&frame, width, height, rotation)?)
        };
        let (width, height) = if let Some(picture) = &transformed {
            chroma = match picture.subsampling {
                Some([2, 2]) => "420",
                Some([2, 1]) => "422",
                Some([1, 1]) => "444",
                _ => {
                    return Err(invalid(
                        "Y4M export cannot represent this transformed chroma layout",
                    ));
                }
            };
            (picture.width, picture.height)
        } else {
            (width, height)
        };
        let current = (width, height, chroma, depth, full, aspect);
        if let Some(first) = layout {
            if first != current {
                return Err(invalid(
                    "Y4M export requires fixed frame geometry and depth",
                ));
            }
        } else {
            layout = Some(current);
            let divisor = gcd(u128::from(scale), duration);
            let tag = if depth == 8 {
                chroma.to_string()
            } else {
                format!("{chroma}p{depth}")
            };
            // The native reader expresses aspect after the display rotation.
            let (an, ad) = aspect;
            writeln!(
                output,
                "YUV4MPEG2 W{width} H{height} F{}:{} Ip A{an}:{ad} C{tag} XCOLORRANGE={}",
                u128::from(scale) / divisor,
                duration / divisor,
                if full { "FULL" } else { "LIMITED" }
            )?;
        }
        output.write_all(b"FRAME\n")?;
        if let Some(picture) = transformed {
            output.write_all(&picture.data)?;
        } else {
            match frame {
                RawFrame::Avc { picture, .. } => picture.write_planar(&mut output)?,
                RawFrame::Planar(p) => output.write_all(&p.frame.data)?,
                RawFrame::Planar8(p) => {
                    output.write_all(&p.y)?;
                    output.write_all(&p.cb)?;
                    output.write_all(&p.cr)?;
                }
                _ => unreachable!(),
            }
        }
        let cw=if chroma=="444" {width} else {width.div_ceil(2)};
        let ch=if chroma=="420" {height.div_ceil(2)} else {height};
        let bytes=width.checked_mul(height).and_then(|n|cw.checked_mul(ch).and_then(|c|c.checked_mul(2)).and_then(|c|n.checked_add(c))).and_then(|n|n.checked_mul(if depth==8 {1} else {2})).ok_or_else(||invalid("Y4M payload count overflow"))?;
        payload_bytes=payload_bytes.checked_add(bytes as u64).ok_or_else(||invalid("Y4M payload count overflow"))?;
        control.packet(bytes)?;
        count += 1;
        if count % 256 != 0 {control.emit(false);}
        control.check()?;
    }
    if count==segment_start {return Err(invalid("input has no decoded video frames"));}
    }
    if count == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    // Both paths share a directory. Unlike rename, hard_link cannot clobber a
    // destination created concurrently; unsupported filesystems return an error.
    control.check()?;
    std::fs::hard_link(&temporary.0, destination)?;
    control.emit(true);
    Ok(crate::media_control::ProgressEvent {packets:count,payload_bytes,done:true})
}
pub(crate) fn transformed_aspect(
    aspect: (u32, u32),
    width: usize,
    height: usize,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<(u32, u32)> {
    let (mut n, mut d) = (u128::from(aspect.0), u128::from(aspect.1));
    let [_, _, mut w, mut h] = geometry.crop.unwrap_or([0, 0, width, height]);
    if geometry.transpose.is_some() {
        std::mem::swap(&mut n, &mut d);
        std::mem::swap(&mut w, &mut h);
    }
    if let Some([pw, ph, _, _]) = geometry.pad {
        w = pw;
        h = ph;
    }
    if let Some([ow, oh]) = geometry.scale {
        n = product(product(n, w as u128)?, oh as u128)?;
        d = product(product(d, h as u128)?, ow as u128)?;
    }
    if n == 0 || d == 0 {
        return Err(invalid("invalid transformed pixel aspect"));
    }
    let divisor = gcd(n, d);
    Ok((
        u32::try_from(n / divisor).map_err(|_| invalid("pixel aspect overflow"))?,
        u32::try_from(d / divisor).map_err(|_| invalid("pixel aspect overflow"))?,
    ))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn product(a: u128, b: u128) -> Result<u128> {
    a.checked_mul(b)
        .ok_or_else(|| invalid("Y4M timestamp overflow"))
}

/// Export ADTS AAC to raw f32le or float WAV, publishing only a complete decode.
pub fn export_aac_pcm(source: &Path, destination: &Path) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_interval(source, destination, None)
}

/// Export a half-open interval from the ADTS decoded sample timeline.
pub fn export_aac_pcm_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_with_volume(source, destination, interval, 1.0)
}

/// Decode AAC to PCM/WAV with finite linear gain after interval selection.
pub fn export_aac_pcm_with_volume(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_transformed(source, destination, interval, volume, None)
}

/// Native mono/stereo conversion. Unchanged channel layouts pass through.
pub fn export_aac_pcm_transformed(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_resampled(source, destination, interval, volume, channels, None)
}

/// Native AAC export including band-limited sample-rate conversion.
pub fn export_aac_pcm_resampled(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_controlled(source, destination, interval, volume, channels, sample_rate, None, None)
}

/// AAC PCM export with encoded-packet progress and cancellation. Counts include
/// reference pre-roll/replayed edits; done is emitted only after publication.
pub fn export_aac_pcm_controlled(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_selected(source, destination, interval, volume, channels, sample_rate, None, cancel, progress)
}

/// Export one explicitly selected container audio stream (zero-based index).
pub fn export_aac_pcm_selected(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(source,destination,interval,volume,channels,sample_rate,selected,cancel,progress,false)
}

/// Export owned AAC, MP4 ALAC or packed RIFF/WAVE PCM through the shared PCM pipeline.
pub fn export_audio_pcm_selected(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(source,destination,interval,volume,channels,sample_rate,selected,cancel,progress,true)
}

fn export_pcm_selected(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
    allow_wave:bool,
) -> Result<crate::native_media::AudioDecodeStats> {
    let mut control = crate::native_media::DecodeProgress::new(cancel, progress)?;
    if sample_rate.is_some_and(|rate| !(8000..=384000).contains(&rate)) {
        return Err(invalid("sample rate must be within 8000..=384000"));
    }
    if !volume.is_finite() || !(0.0..=64.0).contains(&volume) {
        return Err(invalid("volume must be a finite linear gain within 0..=64"));
    }
    use std::io::{Read, Seek, SeekFrom};
    let wav = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => true,
        Some("f32le") => false,
        _ => return Err(invalid("native PCM output requires .f32le or .wav extension")),
    };
    let mut input = BufReader::new(File::open(source)?);
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    let (mp4, matroska, adts, wave) = if allow_wave && matches!(&prefix[..4],b"RIFF"|b"RF64"|b"RIFX") {
        if selected.is_some_and(|n|n!=0) {return Err(invalid("WAVE has only stream 0"));}
        let info=crate::native_pcm::inspect(&mut input,cancel)?;
        info.validate_decode()?;
        (None,None,None,Some((input,info)))
    } else if &prefix[4..8] == b"ftyp" {
        (Some(crate::container::mp4::Mp4Reader::open(input, Default::default())?), None, None,None)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        (None, Some(crate::container::webm::WebmReader::open(input, Default::default())?), None,None)
    } else {
        (None, None, Some(crate::container::adts::StreamReader::open(input)?),None)
    };
    let (input_rate, input_channels) = if let Some(reader) = &mp4 {
        let index = if allow_wave {crate::native_media::mp4_audio_index(reader,selected)?} else {crate::native_media::mp4_aac_index(reader,selected)?};
        let decoder=crate::native_audio_decoder::Mp4PcmDecoder::new(&reader.tracks()[index])?;
        (decoder.sample_rate(),decoder.channels())
    } else if let Some(reader) = &matroska {
        let index = crate::native_media::matroska_aac_index(reader, selected)?;
        let decoder = crate::codec::aac_native::NativeAacDecoder::new(&reader.tracks[index].codec_private)?;
        (decoder.sample_rate(), u16::from(decoder.channels()))
    } else if let Some((_,info))=&wave {
        (info.sample_rate,info.channels)
    } else {
        if selected.is_some_and(|index| index != 0) { return Err(invalid("ADTS has only stream 0")); }
        let config = adts.as_ref().ok_or_else(|| invalid("missing ADTS reader"))?.configuration();
        (config.sample_rate, config.channels)
    };
    let output_rate = sample_rate.unwrap_or(input_rate);
    let output_channels = channels.unwrap_or(input_channels);
    if output_channels != input_channels && !matches!(output_channels, 1 | 2) {
        return Err(invalid("native audio channel conversion supports mono or stereo output"));
    }
    if output_channels != input_channels {
        if input_channels > 6 { return Err(invalid("native audio rematrixing supports 1..=6 input channels")); }
        if let Some((_, info)) = &wave { info.validate_rematrix()?; }
    }
    let output_mask = match &wave {
        Some((_, info)) if output_channels == input_channels => info.channel_mask,
        _ => default_pcm_mask(output_channels)?,
    };
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-pcm-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        }
    }).ok_or_else(|| invalid("cannot reserve PCM output"))??;
    let mut output = BufWriter::new(file);
    if wav { output.write_all(&[0; 80])?; }
    let mut resampler = crate::pcm_resample::Resampler::new(&mut output, input_rate, output_rate, output_channels)?;
    let mut pcm = PcmGain { output: &mut resampler, gain: volume as f32, input_channels, output_channels, frame: [0.0; 6], filled: 0 };
    let mut stats = if let Some(reader) = mp4 {
        crate::native_media::decode_mp4_audio_reader_controlled(reader, &mut pcm, interval, selected, &mut control)?
    } else if let Some(reader) = matroska {
        crate::native_media::decode_matroska_aac_reader_controlled(reader, &mut pcm, interval, selected, &mut control)?
    } else if let Some((reader,info))=wave {
        crate::native_pcm::decode_reader(reader,info,&mut pcm,interval,&mut control)?
    } else {
        crate::native_media::decode_adts_aac_reader_controlled(adts.ok_or_else(|| invalid("missing ADTS reader"))?, &mut pcm, interval, &mut control)?
    };
    control.emit(false);
    control.check()?;
    if pcm.filled != 0 { return Err(invalid("incomplete decoded audio frame")); }
    stats.channels = output_channels;
    stats.sample_frames = resampler.finish()?;
    stats.sample_rate = output_rate;
    if wav {
        let header = float_wav_header_with_mask(&stats, output_mask)?;
        output.seek(SeekFrom::Start(0))?;
        output.write_all(&header)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    control.check()?;
    std::fs::hard_link(&temporary.0, destination)?;
    control.emit(true);
    Ok(stats)
}

pub(crate) fn default_pcm_mask(channels:u16) -> Result<u32> {
    Ok(match channels {
        1 => 0x4, 2 => 0x3, 3 => 0x7, 4 => 0x107, 5 => 0x37, 6 => 0x3f,
        7..=64 => 0,
        _ => return Err(invalid("unsupported WAV channel count")),
    })
}
pub(crate) fn float_wav_header_with_mask(stats: &crate::native_media::AudioDecodeStats,mask:u32) -> Result<Vec<u8>> {
    if !(1..=64).contains(&stats.channels) || (mask != 0 && mask.count_ones()!=u32::from(stats.channels)) {return Err(invalid("invalid WAV channel mask"));}
    let align = stats.channels * 4;
    let bytes = stats.sample_frames.checked_mul(u64::from(align))
        .and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= u32::MAX - 72)
        .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
    let rate = stats.sample_rate.checked_mul(u32::from(align))
        .ok_or_else(|| invalid("WAV byte rate overflow"))?;
    let mut header = Vec::with_capacity(80);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(bytes + 72).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&40u32.to_le_bytes());
    header.extend_from_slice(&0xfffeu16.to_le_bytes());
    header.extend_from_slice(&stats.channels.to_le_bytes());
    header.extend_from_slice(&stats.sample_rate.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&align.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&22u16.to_le_bytes());
    header.extend_from_slice(&32u16.to_le_bytes());
    header.extend_from_slice(&mask.to_le_bytes());
    header.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
    header.extend_from_slice(b"fact");
    header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&(stats.sample_frames as u32).to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&bytes.to_le_bytes());
    Ok(header)
}

// Native decoder writes may split channel frames, but always contain whole samples.
struct PcmGain<'a, W> {
    output: &'a mut W,
    gain: f32,
    input_channels: u16,
    output_channels: u16,
    frame: [f32; 6],
    filled: usize,
}
impl<W: Write> Write for PcmGain<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() % 4 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unaligned PCM write",
            ));
        }
        if self.input_channels == self.output_channels {
            for bytes in data.chunks_exact(4) {
                let value = f32::from_le_bytes(bytes.try_into().unwrap()) * self.gain;
                if !value.is_finite() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PCM gain overflow",
                    ));
                }
                self.output.write_all(&value.to_le_bytes())?;
                self.filled = (self.filled + 1) % usize::from(self.input_channels);
            }
            return Ok(data.len());
        }
        for bytes in data.chunks_exact(4) {
            self.frame[self.filled] = f32::from_le_bytes(bytes.try_into().unwrap());
            self.filled += 1;
            if self.filled != usize::from(self.input_channels) {
                continue;
            }
            let frame = &self.frame;
            let mut mixed = [0.0; 6];
            // Standard decoded order: FL FR FC [LFE] BL BR or BC.
            // LFE is omitted. Centre/surround contributions use -3 dB.
            let k = std::f32::consts::FRAC_1_SQRT_2;
            let (mut left, mut right) = if self.input_channels == 1 {
                (frame[0], frame[0])
            } else {
                (frame[0], frame[1])
            };
            if self.input_channels >= 3 {
                left += k * frame[2];
                right += k * frame[2];
            }
            match self.input_channels {
                4 => {
                    left += k * frame[3];
                    right += k * frame[3];
                }
                5 => {
                    left += k * frame[3];
                    right += k * frame[4];
                }
                6 => {
                    left += k * frame[4];
                    right += k * frame[5];
                }
                _ => {}
            }
            if self.output_channels == 1 {
                mixed[0] = (left + right) * 0.5;
            } else {
                mixed[0] = left;
                mixed[1] = right;
            }
            for sample in &mixed[..usize::from(self.output_channels)] {
                let value = sample * self.gain;
                if !value.is_finite() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PCM gain overflow",
                    ));
                }
                self.output.write_all(&value.to_le_bytes())?;
            }
            self.filled = 0;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

/// Lossless ADTS packet remux with atomic no-overwrite publication.
/// .mka/.mkv selects Matroska; other destination names retain the MP4 contract.
pub fn remux_adts_aac(source: &Path, destination: &Path) -> Result<u64> {
    remux_adts_aac_controlled(source, destination, None, None)
}

/// Native remux with packet progress and cancellation. `done=true` is emitted
/// only after the complete output has been synced and published without overwrite.
pub fn remux_adts_aac_controlled(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<u64> {
    Ok(remux_adts_aac_stats(source, destination, cancel, progress)?.packets)
}

/// Native remux counters returned after successful atomic publication.
pub fn remux_adts_aac_stats(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    let input = crate::container::adts::StreamReader::open(BufReader::new(File::open(source)?))?;
    publish_adts_readers(vec![input],destination,cancel,progress)
}

/// Content detection for the owned ADTS concat route.
pub fn is_adts_source(source: &Path) -> Result<bool> {
    use std::io::Read;
    let mut input=File::open(source)?;
    let mut header=[0;7];
    match input.read_exact(&mut header) {
        Ok(())=>Ok(crate::container::adts::header(&header).is_some()),
        Err(e) if e.kind()==std::io::ErrorKind::UnexpectedEof=>Ok(false),
        Err(e)=>Err(e.into()),
    }
}

/// Copy ADTS segments into one MP4 or Matroska track without decoding. Encoder
/// priming/padding is retained; this is packet concatenation, not gapless editing.
pub fn concat_adts_aac(sources: &[std::path::PathBuf], destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    if !(2..=256).contains(&sources.len()) {return Err(invalid("concat requires 2..=256 inputs"));}
    if !matches!(destination.extension().and_then(|s|s.to_str()),Some("mp4"|"m4a"|"mka"|"mkv")) {return Err(invalid("native ADTS concat output requires .mp4/.m4a or .mka/.mkv"));}
    let mut readers=Vec::with_capacity(sources.len());
    for source in sources {
        if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
        readers.push(crate::container::adts::StreamReader::open(BufReader::new(File::open(source)?))?);
    }
    publish_adts_readers(readers,destination,cancel,progress)
}

fn publish_adts_readers(mut readers: Vec<crate::container::adts::StreamReader<BufReader<File>>>, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-mp4-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => Some(Err(error)),
        }
    }).ok_or_else(|| invalid("cannot reserve MP4 output"))??;
    let mut output = BufWriter::new(file);
    let event = if matches!(destination.extension().and_then(|s|s.to_str()),Some("mka"|"mkv")) {
        if readers.len()==1 {crate::container::matroska_write::write_adts(readers.remove(0), &mut output, cancel, progress)?}
        else {crate::container::matroska_write::concat_adts(readers, &mut output, cancel, progress)?}
    } else if readers.len()==1 {
        crate::container::mp4_write::write_adts_aac_reader_controlled(readers.remove(0), &mut output, cancel, progress)?
    } else {
        crate::container::mp4_write::concat_adts_readers(readers, &mut output, cancel, progress)?
    };
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0, destination)?;
    if let Some(hook) = progress { hook.emit(crate::media_control::ProgressEvent { done: true, ..event }); }
    Ok(crate::media_control::ProgressEvent { done: true, ..event })
}

/// Stream MP4 into fast-start layout without changing packets. Already
/// initialized fragmented MP4 is copied unchanged; fragment relocation is rejected.
pub fn remux_mp4(source: &Path, destination: &Path) -> Result<()> {
    remux_mp4_controlled(source, destination, None, None)
}

/// Byte progress and cancellation for native MP4 relocation. Only mdat payload
/// bytes are counted; packets remain zero. Done is emitted after publication.
pub fn remux_mp4_controlled(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<()> {
    remux_mp4_stats(source, destination, cancel, progress).map(|_| ())
}

/// MP4 relocation counters: mdat bytes, with packets zero (not inspected).
pub fn remux_mp4_stats(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    let mut input=BufReader::new(File::open(source)?);
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_| {
        let path=directory.join(format!(".fvid-relocate-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file)=>Some(Ok((Temporary(path),file))),
            Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>None,
            Err(error)=>Some(Err(error)),
        }
    }).ok_or_else(||invalid("cannot reserve MP4 output"))??;
    let mut output=BufWriter::new(file);
    let event = crate::container::mp4_relocate::fast_start_controlled(&mut input, &mut output, cancel, progress)?;
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0,destination)?;
    if let Some(hook) = progress { hook.emit(crate::media_control::ProgressEvent { done: true, ..event }); }
    Ok(crate::media_control::ProgressEvent { done: true, ..event })
}

/// Trim ADTS audio to float WAVE, decoding pre-roll from the beginning so AAC
/// overlap state is retained. Select sample starts in [from,to), in microseconds.
pub fn trim_adts_wave(source: &Path, destination: &Path, from: i64, to: i64,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str()) != Some("wav") {return Err(invalid("native ADTS trim output requires .wav"));}
    if !is_adts_source(source)? {return Err(invalid("ADTS input required"));}
    export_aac_pcm_selected(source,destination,Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),1.0,None,None,None,cancel,progress)
}

/// Decode a selected AAC presentation interval from ADTS, MP4 or Matroska to WAVE.
/// Multiple container tracks require explicit selection to avoid silent loss.
pub fn trim_aac_wave(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str()) != Some("wav") {return Err(invalid("native AAC trim output requires .wav"));}
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    let index=crate::native_plan::aac_trim_selection(source,selected).map_err(|e|invalid(&e))?;
    export_aac_pcm_selected(source,destination,Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),1.0,None,None,Some(index),cancel,progress)
}

/// Decode video reference pre-roll and retain presentation starts in [from,to).
/// Bounds are relative to the first presented frame. Output retains source cadence.
pub fn trim_y4m(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str())!=Some("y4m") {return Err(invalid("native video trim output requires .y4m"));}
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    validate_video_selection(source,selected)?;
    export_y4m_sources(&[source.to_owned()],destination,Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),&Default::default(),true,cancel,progress)
}

/// Whether the MP4 contains exactly one AAC track and no omitted tracks.
/// A single contiguous media edit is eligible; packet validation occurs during remux.
pub fn is_single_track_mp4_aac(source: &Path) -> Result<bool> {
    let input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    Ok(input.refused().is_empty() && input.tracks().len() == 1
        && input.tracks()[0].handler == *b"soun" && input.tracks()[0].codec == *b"mp4a"
        && (input.tracks()[0].edits.is_empty() || (input.tracks()[0].edits.len() == 1
            && input.tracks()[0].edits[0].media_time >= 0)))
}

/// Publish a single-track AAC MP4 as Matroska using owned packets, edits,
/// file tags and chapters. Existing outputs are never replaced.
pub fn remux_mp4_aac_matroska(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_mp4_matroska_inner(source, destination, cancel, progress, true)
}

/// Select the owned AVC/HEVC/AAC Matroska remux route without discarding tracks.
pub fn is_native_mp4_matroska(source: &Path) -> Result<bool> {
    let input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    Ok(crate::container::mp4_matroska::eligible(&input))
}

/// Publish all supported MP4 video/audio tracks as Matroska, with metadata and edits.
pub fn remux_mp4_matroska(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_mp4_matroska_inner(source, destination, cancel, progress, false)
}
fn remux_mp4_matroska_inner(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>, aac_only: bool,
) -> Result<crate::media_control::ProgressEvent> {
    if cancel.is_some_and(|c| c.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    if !matches!(destination.extension().and_then(|s| s.to_str()), Some("mka" | "mkv")) {
        return Err(invalid("Matroska remux output requires .mka or .mkv"));
    }
    let mut input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-mka-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => Some(Err(error)),
        }
    }).ok_or_else(|| invalid("cannot reserve Matroska output"))??;
    let mut output = BufWriter::new(file);
    let event = if aac_only {
        crate::container::matroska_write::write_mp4_aac_file(&mut input, &mut output, cancel, progress)?
    } else {
        crate::container::mp4_matroska::write(&mut input, &mut output, cancel, progress)?
    };
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|c| c.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0, destination)?;
    let event = crate::media_control::ProgressEvent { done: true, ..event };
    if let Some(hook) = progress { hook.emit(event); }
    Ok(event)
}

/// Decode MP4 video to owned FFV1 and copy all supported AAC companions.
/// Destination is atomically published only after successful flush and sync.
pub fn transcode_mp4_ffv1(source:&Path,destination:&Path,cancel:Option<&crate::media_control::CancelFlag>,progress:Option<&crate::media_control::ProgressHook>)->Result<crate::media_info::LosslessStats> {
    transcode_mp4_ffv1_transformed(source,destination,&Default::default(),&Default::default(),cancel,progress)
}

/// Spatial FFV1 export with the same atomic publication guarantees.
pub fn transcode_ffv1_transformed(source:&Path,destination:&Path,
    geometry:&crate::native_geometry::VideoGeometry,filters:&crate::native_pixels::PixelFilters,
    cancel:Option<&crate::media_control::CancelFlag>,progress:Option<&crate::media_control::ProgressHook>,
)->Result<crate::media_info::LosslessStats> {
    if destination.extension().and_then(|s|s.to_str())!=Some("mkv"){return Err(invalid("lossless export requires FFV1 in .mkv"));}
    if cancel.is_some_and(|c|c.is_cancelled()){return Err(invalid("media operation cancelled"));}
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_|{
        let path=directory.join(format!(".fvid-ffv1-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path){Ok(file)=>Some(Ok((Temporary(path),file))),Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>None,Err(error)=>Some(Err(error))}
    }).ok_or_else(||invalid("cannot reserve FFV1 output"))??;
    let mut output=BufWriter::new(file);
    let (stats,event)=if crate::native_lossless_y4m::is_source(source)? {
        crate::native_lossless_y4m::write(source,&mut output,geometry,filters,cancel,progress)?
    } else { crate::native_lossless::write_mp4_transformed(source,&mut output,geometry,filters,cancel,progress)? };
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|c|c.is_cancelled()){return Err(invalid("media operation cancelled"));}
    std::fs::hard_link(&temporary.0,destination)?;
    if let Some(hook)=progress{hook.emit(crate::media_control::ProgressEvent{done:true,..event});}
    Ok(stats)
}


/// Recognize the EBML signature; the copy operation validates the Matroska document.
pub fn is_matroska_source(source: &Path) -> Result<bool> {
    use std::io::Read;
    let mut input=File::open(source)?;
    let mut signature=[0;4];
    match input.read_exact(&mut signature) {
        Ok(())=>Ok(signature==[0x1a,0x45,0xdf,0xa3]),
        Err(e) if e.kind()==std::io::ErrorKind::UnexpectedEof=>Ok(false),
        Err(e)=>Err(e.into()),
    }
}

/// Identity Matroska remux: retain every track, metadata element and payload.
pub fn remux_matroska(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let audio_only=match destination.extension().and_then(|s|s.to_str()) {
        Some("mkv")=>false, Some("mka")=>true,
        _=>return Err(invalid("native Matroska copy output requires .mkv or .mka")),
    };
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    let mut input=BufReader::new(File::open(source)?);
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_| {
        let path=directory.join(format!(".fvid-matroska-copy-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file)=>Some(Ok((Temporary(path),file))),
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>None,
            Err(e)=>Some(Err(e)),
        }
    }).ok_or_else(||invalid("cannot reserve Matroska copy output"))??;
    let mut output=BufWriter::new(file);
    let event=crate::container::matroska_copy::copy(&mut input,&mut output,audio_only,cancel,progress)?;
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    std::fs::hard_link(&temporary.0,destination)?;
    let event=crate::media_control::ProgressEvent {done:true,..event};
    if let Some(hook)=progress {hook.emit(event);}
    Ok(event)
}

/// Compatibility name retained for callers of the original MP4-only exporter.
pub use transcode_ffv1_transformed as transcode_mp4_ffv1_transformed;
