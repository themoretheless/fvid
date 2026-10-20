//! Byte-preserving contiguous AAC/Opus windows with complete decoder preroll.
use crate::owned_webm::WebmReader;
use fvid_control::CopyOptions;
use std::io::{Read, Seek};
type Result<T> = std::result::Result<T, String>;
pub(super) fn apply<R: Read + Seek>(
    reader: &mut WebmReader<R>,
    index: usize,
    origin: i64,
    window: (i64, i64),
    options: &CopyOptions,
) -> Result<Option<u64>> {
    let track = reader.tracks[index].clone();
    let indices: Vec<_> = reader
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == track.number)
        .map(|(i, _)| i)
        .collect();
    if indices.is_empty() {
        return Ok(Some(0));
    }
    let rate = u32::try_from(track.sample_rate).map_err(|_| "audio rate overflow")?;
    if rate == 0 {
        return Err("zero audio clock".into());
    }
    let samples = |ns: u64| -> Result<u64> {
        u64::try_from((u128::from(ns) * u128::from(rate) + 500_000_000) / 1_000_000_000)
            .map_err(|_| "audio sample overflow".into())
    };
    let ns = |n: u64| -> Result<u64> {
        u64::try_from((u128::from(n) * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate))
            .map_err(|_| "audio timestamp overflow".into())
    };
    let aac_frame = if track.codec == "A_AAC" {
        let config = crate::owned_aac::config::AudioSpecificConfig::parse(&track.codec_private)
            .map_err(|e| e.to_string())?;
        let output = config
            .resolve_output_rate(rate)
            .map_err(|e| e.to_string())?;
        let scaled = u64::from(config.core.frame_samples) * u64::from(output);
        if scaled % u64::from(config.core.sample_rate) != 0 {
            return Err("AAC window frame clock is not sample aligned".into());
        }
        Some(scaled / u64::from(config.core.sample_rate))
    } else {
        None
    };
    if track.codec == "A_OPUS" {
        crate::owned_opus_packet::header_channels(&track.codec_private)
            .map_err(|e| e.to_string())?;
    }
    let first = reader.packets[indices[0]].pts_ns;
    let mut total = 0u64;
    let mut positions = Vec::new();
    let tolerance =
        (u128::from(reader.timestamp_scale_ns()) * u128::from(rate) / 1_000_000_000) as u64;
    for (ordinal, &i) in indices.iter().enumerate() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let p = reader.packets[i].clone();
        let relative = u64::try_from(i128::from(p.pts_ns) - i128::from(first))
            .map_err(|_| "audio window requires ordered timestamps")?;
        if samples(relative)?.abs_diff(total) > tolerance {
            return Ok(None);
        }
        if p.discard_padding_ns != 0
            && !((ordinal == 0 && p.discard_padding_ns < 0)
                || (ordinal + 1 == indices.len() && p.discard_padding_ns > 0))
        {
            return Ok(None);
        }
        let frames = if let Some(frame) = aac_frame {
            frame
        } else {
            samples(
                crate::owned_opus_packet::duration_ns_with_header(
                    &track.codec_private,
                    &reader.read_packet(i).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
            )?
        };
        positions.push((i, total, frames));
        total = total
            .checked_add(frames)
            .ok_or("audio window sample overflow")?;
    }
    let head = samples(
        reader.packets[indices[0]]
            .discard_padding_ns
            .min(0)
            .unsigned_abs(),
    )?;
    let initial = samples(track.codec_delay_ns)?.max(head);
    let tail = samples(
        reader.packets[*indices.last().unwrap()]
            .discard_padding_ns
            .max(0) as u64,
    )?;
    let audible_end = total
        .checked_sub(tail)
        .ok_or("audio padding exceeds stream")?;
    if initial > audible_end {
        return Err("audio delay exceeds stream".into());
    }
    let boundary = |us: i64| -> Result<u64> {
        let offset = i128::from(origin) + i128::from(us) * 1000 - i128::from(first);
        let n = u128::try_from(offset.max(0)).map_err(|_| "audio window time overflow")?;
        let extra = u64::try_from((n * u128::from(rate)).div_ceil(1_000_000_000))
            .map_err(|_| "audio window sample overflow")?;
        initial
            .checked_add(extra)
            .ok_or_else(|| "audio window sample overflow".into())
    };
    let begin = boundary(window.0)?.min(audible_end);
    let end = boundary(window.1)?.min(audible_end);
    let lead = u64::try_from(
        (i128::from(first) - i128::from(origin) - i128::from(window.0) * 1000).max(0),
    )
    .map_err(|_| "audio window timestamp overflow")?;
    reader.tracks[index].codec_delay_ns = if track.codec == "A_OPUS" {
        if rate != 48000 {
            return Err("Opus trim requires the 48 kHz Matroska clock".into());
        }
        let Ok(pre_skip) = u16::try_from(begin) else {
            return Ok(None);
        };
        reader.tracks[index].codec_private[10..12].copy_from_slice(&pre_skip.to_le_bytes());
        crate::owned_opus_packet::pre_skip_ns(&reader.tracks[index].codec_private)
            .map_err(|e| e.to_string())?
    } else {
        ns(begin)?
    };
    for (i, start, frames) in positions {
        let p = &mut reader.packets[i];
        if begin >= end || start >= end {
            p.track = 0;
            continue;
        }
        let finish = start.checked_add(frames).ok_or("audio sample overflow")?;
        p.pts_ns = i64::try_from(
            ns(start)?
                .checked_add(lead)
                .ok_or("audio window timestamp overflow")?,
        )
        .map_err(|_| "audio timestamp overflow")?;
        p.duration_ns = Some(ns(finish)? - ns(start)?);
        p.discard_padding_ns = if finish > end {
            i64::try_from(ns(finish - end)?).map_err(|_| "audio padding overflow")?
        } else {
            0
        };
    }
    Ok(Some(
        (audible_end - initial).saturating_sub(end.saturating_sub(begin)),
    ))
}
