//! Strict stream-copy editing. Reject unsupported boundaries rather than silently reencode.
use super::*;

/// Decimal seconds, parsed exactly into microseconds (no floating-point rounding).
pub fn parse_time(text: &str) -> Result<i64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(
            "time must be nonnegative decimal seconds with at most 6 fractional digits".into(),
        );
    }
    let whole = whole.parse::<i64>().map_err(|_| "time overflow")?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().map_err(|_| "invalid fraction")?
            * 10i64.pow(6 - fraction.len() as u32)
    };
    whole
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(fraction))
        .ok_or_else(|| "time overflow".into())
}
fn ticks(us: i64, tb: AVRational) -> Result<i64> {
    if tb.num <= 0 || tb.den <= 0 {
        return Err("invalid time base".into());
    }
    let n = i128::from(us) * i128::from(tb.den);
    let d = 1_000_000i128 * i128::from(tb.num);
    if n % d != 0 {
        return Err("requested time is not exactly representable in stream time base".into());
    }
    i64::try_from(n / d).map_err(|_| "timestamp overflow".into())
}
fn same_time(a: i64, at: AVRational, b: i64, bt: AVRational) -> bool {
    i128::from(a) * i128::from(at.num) * i128::from(bt.den)
        == i128::from(b) * i128::from(bt.num) * i128::from(at.den)
}
fn compatible(a: &Input, b: &Input, selected: &[usize]) -> Result<()> {
    if a.streams().len() != b.streams().len() {
        return Err("concat stream counts differ".into());
    }
    // SAFETY: Both guards retain the full codec parameter trees throughout comparison.
    unsafe {
        for &index in selected {
            let sa = &*a.streams()[index];
            let sb = &*b.streams()[index];
            let x = &*sa.codecpar;
            let y = &*sb.codecpar;
            let layout = av_channel_layout_compare(&x.ch_layout, &y.ch_layout);
            if x.codec_type != y.codec_type
                || x.codec_id != y.codec_id
                || x.format != y.format
                || x.width != y.width
                || x.height != y.height
                || x.profile != y.profile
                || x.level != y.level
                || x.sample_rate != y.sample_rate
                || layout != 0
                || x.bits_per_coded_sample != y.bits_per_coded_sample
                || x.bits_per_raw_sample != y.bits_per_raw_sample
                || x.color_range != y.color_range
                || x.color_primaries != y.color_primaries
                || x.color_trc != y.color_trc
                || x.color_space != y.color_space
                || x.chroma_location != y.chroma_location
                || x.sample_aspect_ratio.num != y.sample_aspect_ratio.num
                || x.sample_aspect_ratio.den != y.sample_aspect_ratio.den
                || x.initial_padding != 0
                || y.initial_padding != 0
                || x.trailing_padding != 0
                || y.trailing_padding != 0
                || x.video_delay != 0
                || y.video_delay != 0
                || sa.time_base.num != sb.time_base.num
                || sa.time_base.den != sb.time_base.den
            {
                return Err(format!(
                    "concat stream {index} codec/layout/timebase differs or has delay/padding"
                ));
            }
            if x.nb_coded_side_data != 0 || y.nb_coded_side_data != 0 {
                return Err("concat of codec side data is not qualified".into());
            }
            if x.extradata_size != y.extradata_size
                || (x.extradata_size > 0
                    && slice::from_raw_parts(x.extradata, x.extradata_size as usize)
                        != slice::from_raw_parts(y.extradata, y.extradata_size as usize))
            {
                return Err("concat codec configuration differs".into());
            }
        }
    }
    Ok(())
}
fn independent(packet: &AVPacket, p: &AVCodecParameters) -> Result<()> {
    if p.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO {
        return Ok(());
    }
    if packet.flags & AV_PKT_FLAG_KEY as i32 == 0 {
        return Err("cut/segment must start at an independent video frame".into());
    }
    // FFV1 keyframes reset entropy contexts; non-key FFV1 frames are not
    // automatically independent even though pixel prediction is intra-frame.
    if p.codec_id == AVCodecID_AV_CODEC_ID_FFV1 {
        return Ok(());
    }
    // SAFETY: Descriptor is static; packet and codec parameters are borrowed from live guards.
    unsafe {
        let descriptor = avcodec_descriptor_get(p.codec_id);
        if !descriptor.is_null() && (*descriptor).props & AV_CODEC_PROP_INTRA_ONLY as i32 != 0 {
            return Ok(());
        }
        if p.codec_id == AVCodecID_AV_CODEC_ID_H264 {
            let payload = slice::from_raw_parts(packet.data, packet.size as usize);
            let extra = if p.extradata_size > 0 {
                slice::from_raw_parts(p.extradata, p.extradata_size as usize)
            } else {
                &[]
            };
            if h264_idr(payload, extra) {
                return Ok(());
            }
        }
    }
    Err("independent boundary not qualified for this codec/GOP; supported: intra-only codecs or H.264 IDR without reordered frames".into())
}
fn h264_idr(payload: &[u8], extra: &[u8]) -> bool {
    if extra.len() >= 5 && extra[0] == 1 {
        let length = (extra[4] & 3) as usize + 1;
        let mut offset = 0;
        while offset + length <= payload.len() {
            let size = payload[offset..offset + length]
                .iter()
                .fold(0usize, |a, &b| (a << 8) | b as usize);
            offset += length;
            if size == 0 || size > payload.len() - offset {
                return false;
            }
            if payload[offset] & 31 == 5 {
                return true;
            }
            offset += size;
        }
        false
    } else {
        payload
            .windows(4)
            .any(|w| w[..3] == [0, 0, 1] && w[3] & 31 == 5)
    }
}
struct Coverage {
    origin: Option<i64>,
    end: Option<i64>,
    tb: AVRational,
}
fn strict_packet(packet: &AVPacket, p: &AVCodecParameters) -> Result<()> {
    if packet.size <= 0 {
        return Err("strict edit rejects empty packets".into());
    }
    if packet.pts == NOPTS || packet.dts == NOPTS || packet.duration <= 0 {
        return Err("strict edit requires PTS, DTS and positive packet duration".into());
    }
    if packet.pts != packet.dts || p.video_delay != 0 {
        return Err(
            "strict edit of reordered/B-frame streams is not qualified; remux remains available"
                .into(),
        );
    }
    if p.initial_padding != 0 || p.trailing_padding != 0 || packet.side_data_elems != 0 {
        return Err("strict edit of delay/padding or packet side data is not qualified".into());
    }
    if p.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO
        && p.codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO
    {
        return Err("strict edit supports selected audio/video streams only".into());
    }
    Ok(())
}
/// Exact packet boundaries; start/end measured from container start. No fallback or frame snapping.
pub fn trim(
    source: &Path,
    destination: &Path,
    from_us: i64,
    to_us: i64,
    options: &CopyOptions,
) -> Result<CopyStats> {
    if from_us < 0 || to_us <= from_us {
        return Err("trim requires 0 <= from < to".into());
    }
    edit(
        &[source.to_path_buf()],
        destination,
        Some((from_us, to_us)),
        options,
    )
}
pub fn concat(sources: &[PathBuf], destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    if sources.len() < 2 || sources.len() > 256 {
        return Err("concat requires 2..=256 inputs".into());
    }
    edit(sources, destination, None, options)
}
fn edit(
    sources: &[PathBuf],
    destination: &Path,
    range: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<CopyStats> {
    let template = Input::open(&sources[0])?;
    let selected = selection(&template, options)?;
    let mut output = Output::new(destination, &template, &selected)?;
    output.strict_timing = true;
    let mut offsets = vec![0i64; selected.len()];
    let mut stats = CopyStats {
        backend: "libavformat (native, strict stream copy)",
        segments: sources.len(),
        ..Default::default()
    };
    for source in sources {
        let mut input = Input::open(source)?;
        // SAFETY: Input owns a valid format context. Editing chapter ranges is a
        // separate operation; reject them rather than publish stale chapter times.
        if unsafe { (*input.0).nb_chapters } != 0 {
            return Err("strict trim/concat of chapters is not qualified; remux and full-duration crop preserve them".into());
        }
        if range.is_none() {
            compatible(&template, &input, &selected)?;
        }
        // SAFETY: Input is live. The format start is expressed in AV_TIME_BASE units.
        let start_us = unsafe {
            if (*input.0).start_time == NOPTS {
                0
            } else {
                (*input.0).start_time
            }
        };
        let mut coverage: Vec<_> = selected
            .iter()
            .map(|&i| {
                // SAFETY: Selected indices were validated against equally-sized stream tables.
                Coverage {
                    origin: None,
                    end: None,
                    tb: unsafe { (*input.streams()[i]).time_base },
                }
            })
            .collect();
        let bounds: Vec<_> = coverage
            .iter()
            .map(|c| {
                range
                    .map(|(a, b)| {
                        let a = a.checked_add(start_us).ok_or("start overflow")?;
                        let b = b.checked_add(start_us).ok_or("end overflow")?;
                        Ok((ticks(a, c.tb)?, ticks(b, c.tb)?))
                    })
                    .transpose()
            })
            .collect::<Result<_>>()?;
        let mut packet = Packet::new()?;
        while packet.read(&mut input)? {
            let (index, size) = packet_info(&packet, &input, options)?;
            let Some(mapped) = selected.iter().position(|&i| i == index) else {
                continue;
            };
            let c = &mut coverage[mapped];
            // SAFETY: Packet and codec parameters are live; only packet timestamps are mutated.
            unsafe {
                let p = &mut *packet.0;
                let codec = &*(*input.streams()[index]).codecpar;
                if let Some((start, end)) = bounds[mapped] {
                    if p.pts == NOPTS {
                        return Err("missing packet PTS".into());
                    }
                    let packet_end = p.pts.checked_add(p.duration).ok_or("timestamp overflow")?;
                    if p.pts < start {
                        if packet_end > start {
                            return Err(
                                "start cuts through a packet; exact stream copy impossible".into(),
                            );
                        }
                        continue;
                    }
                    if p.pts >= end {
                        continue;
                    }
                    if packet_end > end {
                        return Err(
                            "end cuts through a packet; exact stream copy impossible".into()
                        );
                    }
                }
                strict_packet(p, codec)?;
                if c.origin.is_none() {
                    independent(p, codec)?;
                    if let Some((start, _)) = bounds[mapped]
                        && p.pts != start
                    {
                        return Err("requested start has no exact packet boundary".into());
                    }
                    c.origin = Some(p.pts);
                }
                if let Some(previous) = c.end
                    && previous != p.pts
                {
                    return Err("packet gap/overlap rejected by strict editing".into());
                }
                c.end = Some(p.pts.checked_add(p.duration).ok_or("packet end overflow")?);
                let shift = offsets[mapped]
                    .checked_sub(c.origin.unwrap())
                    .ok_or("timeline shift overflow")?;
                p.pts = p.pts.checked_add(shift).ok_or("PTS overflow")?;
                p.dts = p.dts.checked_add(shift).ok_or("DTS overflow")?;
            }
            output.write(&mut packet, mapped, c.tb)?;
            stats.packets += 1;
            stats.payload_bytes += size as u64;
        }
        let mut durations = Vec::new();
        for (i, c) in coverage.iter().enumerate() {
            let first = c.origin.ok_or("selected stream has no packets in range")?;
            let end = c.end.ok_or("selected stream is empty")?;
            if let Some((_, target)) = bounds[i]
                && end != target
            {
                return Err("requested end has no exact packet boundary".into());
            }
            durations.push(end.checked_sub(first).ok_or("duration overflow")?);
            if i > 0
                && (!same_time(first, c.tb, coverage[0].origin.unwrap(), coverage[0].tb)
                    || !same_time(durations[i], c.tb, durations[0], coverage[0].tb))
            {
                return Err("audio/video starts or ends differ; strict concat/trim would create a gap or sync shift".into());
            }
            offsets[i] = offsets[i]
                .checked_add(durations[i])
                .ok_or("concat timeline overflow")?;
        }
    }
    output.finish()?;
    Ok(stats)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_decimal_time() {
        assert_eq!(parse_time("1.25").unwrap(), 1_250_000);
        for s in ["-1", "NaN", "1.0000001", "1e3", "", "1:20"] {
            assert!(parse_time(s).is_err());
        }
        assert!(ticks(1, AVRational { num: 1, den: 25 }).is_err());
    }
    #[test]
    fn idr_boundaries() {
        assert!(h264_idr(&[0, 0, 0, 1, 0x65, 1], &[]));
        assert!(!h264_idr(&[0, 0, 1, 0x41, 1], &[]));
        assert!(h264_idr(&[0, 0, 0, 2, 0x65, 1], &[1, 0, 0, 0, 3]));
        assert!(!h264_idr(&[0, 0, 0, 9, 0x65], &[1, 0, 0, 0, 3]));
    }
}
