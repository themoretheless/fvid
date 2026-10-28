//! Sample-exact complex companion windows staged through owned float32 PCM.
use crate::{owned_matroska as mkv, owned_video_temporary::Directory, owned_webm::Track};
use fvid_control::CopyOptions;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
type Result<T> = std::result::Result<T, String>;
pub(super) struct Prepared {
    pub path: PathBuf,
    pub sample_frames: u64,
    pub rate: u32,
}

pub(super) fn prepare(
    source: &Path,
    index: usize,
    track: &Track,
    first: i64,
    origin: i64,
    window: (i64, i64),
    options: &CopyOptions,
    private: &mut Directory,
) -> Result<Prepared> {
    let mut control = options.clone();
    control.streams = vec![index];
    control.progress = None;
    control.metadata_set.clear();
    control.metadata_delete.clear();
    control.stream_metadata_set.clear();
    control.stream_metadata_delete.clear();
    let start = i128::from(origin) + i128::from(window.0) * 1000;
    let end = i128::from(origin) + i128::from(window.1) * 1000;
    let local_start =
        u64::try_from((start - i128::from(first)).max(0)).map_err(|_| "audio start overflow")?;
    let local_end =
        u64::try_from((end - i128::from(first)).max(0)).map_err(|_| "audio end overflow")?;
    let lead =
        u64::try_from((i128::from(first) - start).max(0)).map_err(|_| "audio lead overflow")?;
    let raw_path = private.stage_path(&format!("audio-{index}.f32"));
    let mut raw = BufWriter::new(File::create(&raw_path).map_err(|e| e.to_string())?);
    let geometry = || -> Result<(u32, u16)> {
        if track.codec == "A_AAC" {
            let (r, c, _) = crate::owned_matroska_aac_export::geometry(source, &control)?;
            Ok((r, c))
        } else {
            crate::owned_matroska_opus_export::geometry(source, &control)
        }
    };
    let (rate, channels, frames) = if local_end > local_start {
        let interval = Some((
            Duration::from_nanos(local_start),
            Duration::from_nanos(local_end),
        ));
        let input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
        let result = if track.codec == "A_AAC" {
            crate::owned_matroska_aac::decode_matroska_aac_pcm(input, &mut raw, interval, &control)
                .map_err(|e| e.to_string())
        } else {
            crate::owned_matroska_opus::decode_matroska_opus_pcm(
                input, &mut raw, interval, &control,
            )
            .map_err(|e| e.to_string())
        };
        match result {
            Ok(stats) => (stats.sample_rate, stats.channels, stats.sample_frames),
            Err(error) if error == "Matroska audio interval contains no samples" => {
                let (r, c) = geometry()?;
                (r, c, 0)
            }
            Err(error) => return Err(error),
        }
    } else {
        let (r, c) = geometry()?;
        (r, c, 0)
    };
    raw.flush().map_err(|e| e.to_string())?;
    drop(raw);
    let frame_bytes = usize::from(channels)
        .checked_mul(4)
        .ok_or("PCM stride overflow")?;
    if frame_bytes == 0 || rate == 0 {
        return Err("invalid PCM geometry".into());
    }
    let packet_frames = (options.max_packet_bytes / frame_bytes).min(1024);
    if packet_frames == 0 {
        return Err("PCM audio frame exceeds packet limit".into());
    }
    let path = private.stage_path(&format!("audio-{index}.mkv"));
    let mut output = File::create(&path).map_err(|e| e.to_string())?;
    let specs = [mkv::TrackSpec {
        encoding: mkv::Encoding::PcmFloat32 {
            sample_rate: rate,
            channels,
        },
        name: &track.name,
        language: &track.language,
    }];
    let mut writer = mkv::PacketWriter::new(&mut output, &specs).map_err(|e| e.to_string())?;
    let mut input = BufReader::new(File::open(raw_path).map_err(|e| e.to_string())?);
    let mut data = crate::owned_frame::buffer(
        packet_frames
            .checked_mul(frame_bytes)
            .ok_or("PCM packet size overflow")?,
    )?;
    let ns = |n: u64| -> Result<u64> {
        u64::try_from((u128::from(n) * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate))
            .map_err(|_| "PCM timestamp overflow".into())
    };
    let mut sent = 0u64;
    while sent < frames {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let count = (frames - sent).min(packet_frames as u64);
        let bytes = count as usize * frame_bytes;
        input
            .read_exact(&mut data[..bytes])
            .map_err(|e| e.to_string())?;
        let begin = ns(sent)?;
        let end = ns(sent + count)?;
        writer
            .write_packet(
                0,
                begin.checked_add(lead).ok_or("PCM timestamp overflow")?,
                end - begin,
                true,
                &data[..bytes],
            )
            .map_err(|e| e.to_string())?;
        sent += count;
    }
    let mut extra = [0];
    if input.read(&mut extra).map_err(|e| e.to_string())? != 0 {
        return Err("PCM stage length disagrees with decoded sample count".into());
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(Prepared {
        path,
        sample_frames: frames,
        rate,
    })
}

/// Mirror the existing presentation scheduler on packet geometry, without
/// decoding the unselected tail just to count discarded samples.
pub(super) fn presentation_samples<R: Read + std::io::Seek>(
    reader: &mut crate::owned_webm::WebmReader<R>,
    index: usize,
    rate: u32,
    options: &CopyOptions,
) -> Result<u64> {
    let track = reader.tracks[index].clone();
    let indices: Vec<_> = reader
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == track.number)
        .map(|(i, _)| i)
        .collect();
    let Some(&first_index) = indices.first() else {
        return Ok(0);
    };
    let base = reader.packets[first_index].pts_ns;
    let samples = |ns: u64| -> Result<u64> {
        u64::try_from((u128::from(ns) * u128::from(rate) + 500_000_000) / 1_000_000_000)
            .map_err(|_| "audio position overflow".into())
    };
    let full_delay = samples(track.codec_delay_ns)?;
    let mut delay = full_delay;
    let mut position = 0u64;
    let mut leading = 0u64;
    let precision =
        (u128::from(reader.timestamp_scale_ns()) * u128::from(rate) / 1_000_000_000) as u64;
    let aac = if track.codec == "A_AAC" {
        let config = crate::owned_aac::config::AudioSpecificConfig::parse(&track.codec_private)
            .map_err(|e| e.to_string())?;
        let n = u64::from(config.core.frame_samples) * u64::from(rate);
        let d = u64::from(config.core.sample_rate);
        if n % d != 0 {
            return Err("AAC presentation frame clock is not sample aligned".into());
        }
        Some(n / d)
    } else {
        None
    };
    for (ordinal, i) in indices.into_iter().enumerate() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let p = reader.packets[i].clone();
        let frames = if let Some(frames) = aac {
            frames
        } else {
            samples(
                crate::owned_opus_packet::duration_ns_with_header(
                    &track.codec_private,
                    &reader.read_packet(i).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
            )?
        };
        let padding = samples(p.discard_padding_ns.unsigned_abs())?;
        if padding > frames {
            return Err("audio padding exceeds packet".into());
        }
        let head = if p.discard_padding_ns < 0 { padding } else { 0 };
        let tail = if p.discard_padding_ns > 0 { padding } else { 0 };
        let skip = delay.min(frames);
        delay -= skip;
        let first = head.max(skip);
        let end = frames - tail;
        if first > end {
            return Err("audio trimming overlaps".into());
        }
        if ordinal == 0 {
            leading = head.saturating_sub(skip);
        }
        let available = end - first;
        if available == 0 {
            continue;
        }
        let timestamp = ((i128::from(p.pts_ns) - i128::from(base)) * i128::from(rate)
            + 500_000_000)
            .div_euclid(1_000_000_000)
            - i128::from(full_delay);
        let start = timestamp + i128::from(first) - i128::from(leading);
        let start = if start < 0 && start.unsigned_abs() <= u128::from(precision) {
            0
        } else {
            u64::try_from(start).map_err(|_| "negative audio presentation timestamp")?
        };
        if start < position && position - start > precision {
            return Err("overlapping audio presentation intervals".into());
        }
        if start > position && start - position > precision {
            position = start;
        }
        position = position
            .checked_add(available)
            .ok_or("audio presentation length overflow")?;
    }
    Ok(position)
}
