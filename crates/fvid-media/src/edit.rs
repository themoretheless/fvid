//! Strict stream-copy editing. Reject unsupported boundaries rather than silently reencode.
use super::*;

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
                || x.initial_padding != y.initial_padding
                || x.trailing_padding != y.trailing_padding
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
        let payload = slice::from_raw_parts(packet.data, packet.size as usize);
        let extra = if p.extradata_size > 0 {
            slice::from_raw_parts(p.extradata, p.extradata_size as usize)
        } else {
            &[]
        };
        if p.codec_id == AVCodecID_AV_CODEC_ID_H264 && h264_idr(payload, extra) {
            return Ok(());
        }
        if p.codec_id == AVCodecID_AV_CODEC_ID_HEVC && hevc_irap(payload, extra) {
            return Ok(());
        }
    }
    Err("independent boundary not qualified for this codec/GOP; supported: intra-only codecs, H.264 IDR, or HEVC IRAP (closed-GOP B-frames allowed between RAP)".into())
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
/// HEVC IRAP NAL types: BLA 16–18, IDR 19–20, CRA 21.
fn hevc_irap(payload: &[u8], extra: &[u8]) -> bool {
    let is_irap = |nal_type: u8| (16..=21).contains(&nal_type);
    if extra.first() == Some(&1) && extra.len() > 21 {
        let length = (extra[21] & 3) as usize + 1;
        let mut offset = 0;
        while offset + length <= payload.len() {
            let size = payload[offset..offset + length]
                .iter()
                .fold(0usize, |a, &b| (a << 8) | b as usize);
            offset += length;
            if size == 0 || size > payload.len() - offset {
                return false;
            }
            if is_irap((payload[offset] >> 1) & 0x3f) {
                return true;
            }
            offset += size;
        }
        false
    } else {
        let mut i = 0;
        while i + 4 <= payload.len() {
            let (start, sc) = if payload[i..].starts_with(&[0, 0, 0, 1]) {
                (i + 4, 4)
            } else if payload[i..].starts_with(&[0, 0, 1]) {
                (i + 3, 3)
            } else {
                i += 1;
                continue;
            };
            let _ = sc;
            if start < payload.len() && is_irap((payload[start] >> 1) & 0x3f) {
                return true;
            }
            i = start;
        }
        false
    }
}
fn keyframe(packet: &AVPacket) -> bool {
    packet.flags & AV_PKT_FLAG_KEY as i32 != 0
}
pub(crate) fn closed_gop_rap(codec: &AVCodecParameters, packet: &AVPacket) -> bool {
    if !keyframe(packet) {
        return false;
    }
    // SAFETY: Packet payload and codec extradata are live for this call.
    unsafe {
        let payload = slice::from_raw_parts(packet.data, packet.size as usize);
        let extra = if codec.extradata_size > 0 {
            slice::from_raw_parts(codec.extradata, codec.extradata_size as usize)
        } else {
            &[]
        };
        match codec.codec_id {
            AVCodecID_AV_CODEC_ID_H264 => h264_idr(payload, extra),
            AVCodecID_AV_CODEC_ID_HEVC => hevc_irap(payload, extra),
            _ => false,
        }
    }
}
struct Coverage {
    /// First included packet PTS (closed-GOP RAP for reorder streams).
    origin: Option<i64>,
    /// Presentation window start: requested trim `from` (may be after `origin` for mid-GOP).
    present_origin: Option<i64>,
    /// Latest closed RAP ≤ `from` from the pre-trim scan; copy must begin here.
    required_rap: Option<i64>,
    /// True once a packet with PTS == present_origin was included.
    saw_present_start: bool,
    /// True once a packet with PTS == requested end was observed (presentation exclusive end).
    saw_present_end: bool,
    end: Option<i64>,
    /// Decode-order cursor used when PTS≠DTS (B-frames).
    dts_end: Option<i64>,
    /// Highest exclusive presentation end among included packets.
    pts_end: Option<i64>,
    tb: AVRational,
    reorder: bool,
    finished: bool,
    /// AAC/MP3/FLAC mid-packet trim: skip stream copy; decode to PCM after the copy loop.
    decode_seam: bool,
}
fn strict_packet(packet: &AVPacket, p: &AVCodecParameters, allow_reorder: bool) -> Result<()> {
    if packet.size <= 0 {
        return Err("strict edit rejects empty packets".into());
    }
    if packet.pts == NOPTS || packet.dts == NOPTS || packet.duration <= 0 {
        return Err("strict edit requires PTS, DTS and positive packet duration".into());
    }
    let reordered = packet.pts != packet.dts || p.video_delay != 0;
    if reordered && !allow_reorder {
        return Err(
            "strict edit of reordered/B-frame streams is not qualified; remux remains available"
                .into(),
        );
    }
    if reordered
        && p.codec_id != AVCodecID_AV_CODEC_ID_H264
        && p.codec_id != AVCodecID_AV_CODEC_ID_HEVC
    {
        return Err(
            "reordered stream-copy trim is qualified only for closed-GOP H.264 IDR or HEVC IRAP boundaries"
                .into(),
        );
    }
    if p.codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO {
        // AAC/MP3 priming uses initial_padding and may attach packet side data; keep both.
    } else if p.initial_padding != 0 || p.trailing_padding != 0 || packet.side_data_elems != 0 {
        return Err("strict edit of delay/padding or packet side data is not qualified".into());
    }
    if p.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO
        && p.codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO
    {
        return Err("strict edit supports selected audio/video streams only".into());
    }
    Ok(())
}
/// Exact packet boundaries for stream-copyable streams. AAC/MP3/FLAC that would be cut
/// mid-packet are decoded to sample-exact PCM in the same output (audio-only or beside
/// video stream copy). PCM mid-packet cuts still require `trim-pcm` / lossless interval.
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
    match edit(
        &[source.to_path_buf()],
        destination,
        Some((from_us, to_us)),
        options,
        false,
    ) {
        Ok(stats) => Ok(stats),
        Err(err)
            if err.contains("cuts through a packet")
                || err.contains("no exact packet boundary") =>
        {
            // Mid-packet or off-boundary AAC/MP3/FLAC: retry with sample-exact PCM seams.
            // If nothing can be decoded (video-only failure), keep the original stream-copy error.
            match edit(
                &[source.to_path_buf()],
                destination,
                Some((from_us, to_us)),
                options,
                true,
            ) {
                Ok(stats) => Ok(stats),
                Err(_) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}
pub fn concat(sources: &[PathBuf], destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    if sources.len() < 2 || sources.len() > 256 {
        return Err("concat requires 2..=256 inputs".into());
    }
    edit(sources, destination, None, options, false)
}
fn edit(
    sources: &[PathBuf],
    destination: &Path,
    range: Option<(i64, i64)>,
    options: &CopyOptions,
    force_audio_decode: bool,
) -> Result<CopyStats> {
    let mut template = Some(Input::open_fast(&sources[0])?);
    let selected = selection(template.as_ref().unwrap(), options)?;
    crate::budget::admit_input_controlled_budget(template.as_ref().unwrap(), options, 1, false)?;
    crate::budget::check_rss_budget(options)?;
    let mut decode_audio: Vec<(usize, usize, i32)> = Vec::new();
    let mut audio_holders = Vec::new();
    let mut overrides: Vec<(usize, *const AVCodecParameters, AVRational)> = Vec::new();
    if force_audio_decode && range.is_some() {
        let input_ref = template.as_ref().unwrap();
        for (mapped, &index) in selected.iter().enumerate() {
            // SAFETY: Selected indices validated against the live stream table.
            let codec = unsafe { &*(*input_ref.streams()[index]).codecpar };
            if codec.codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO {
                continue;
            }
            if pcm::classify_interval_audio(codec).ok() != Some(pcm::IntervalAudio::Decode) {
                continue;
            }
            let (pcm_params, audio_tb, _) = audio::pcm_parameters_for_interval_decode(codec)?;
            let rate = unsafe { (*pcm_params.0).sample_rate };
            overrides.push((index, pcm_params.0 as *const _, audio_tb));
            audio_holders.push(pcm_params);
            decode_audio.push((index, mapped, rate));
        }
        if decode_audio.is_empty() {
            return Err("start cuts through a packet; exact stream copy impossible".into());
        }
    }
    let mut stats = CopyStats {
        backend: if decode_audio.is_empty() {
            "libavformat (native, strict stream copy)"
        } else {
            "libavformat (native, stream copy + compressed-audio seam → PCM)"
        },
        segments: sources.len(),
        ..Default::default()
    };
    let mut output = if overrides.is_empty() {
        Output::new_direct(destination, template.as_ref().unwrap(), &selected, None)?
    } else {
        Output::with_overrides(
            destination,
            template.as_ref().unwrap(),
            &selected,
            &overrides,
            false,
            None,
        )?
    };
    output.strict_timing = decode_audio.is_empty();
    drop(audio_holders);
    let mut offsets = vec![0i64; selected.len()];
    for source in sources {
        let mut input = if sources.len() == 1 {
            template.take().unwrap()
        } else {
            Input::open_fast(source)?
        };
        // SAFETY: Input owns a valid format context. Editing chapter ranges is a
        // separate operation; reject them rather than publish stale chapter times.
        if unsafe { (*input.0).nb_chapters } != 0 {
            return Err("strict trim/concat of chapters is not qualified; remux and full-duration crop preserve them".into());
        }
        if range.is_none() {
            compatible(template.as_ref().unwrap(), &input, &selected)?;
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
            .enumerate()
            .map(|(mapped, &i)| {
                // SAFETY: Selected indices were validated against equally-sized stream tables.
                let codec = unsafe { &*(*input.streams()[i]).codecpar };
                Coverage {
                    origin: None,
                    present_origin: None,
                    required_rap: None,
                    saw_present_start: false,
                    saw_present_end: false,
                    end: None,
                    dts_end: None,
                    pts_end: None,
                    tb: unsafe { (*input.streams()[i]).time_base },
                    reorder: codec.video_delay != 0
                        || codec.codec_id == AVCodecID_AV_CODEC_ID_H264
                        || codec.codec_id == AVCodecID_AV_CODEC_ID_HEVC,
                    finished: false,
                    decode_seam: decode_audio.iter().any(|&(_, m, _)| m == mapped),
                }
            })
            .collect();
        // Reorder is only meaningful for video; force audio to non-reorder coverage.
        for (i, c) in coverage.iter_mut().enumerate() {
            let codec = unsafe { &*(*input.streams()[selected[i]]).codecpar };
            if codec.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO {
                c.reorder = false;
            } else if codec.video_delay == 0 && codec.codec_id != AVCodecID_AV_CODEC_ID_HEVC {
                // Keep H.264 no-B as the contiguous-PTS path. HEVC often leaves
                // video_delay unset in MP4 even when B-frames are present, so keep
                // the closed-GOP IRAP reorder path for HEVC.
                c.reorder = false;
            }
        }
        let bounds: Vec<_> = coverage
            .iter()
            .map(|c| {
                if c.decode_seam {
                    return Ok(None);
                }
                range
                    .map(|(a, b)| {
                        let a = a.checked_add(start_us).ok_or("start overflow")?;
                        let b = b.checked_add(start_us).ok_or("end overflow")?;
                        Ok((ticks(a, c.tb)?, ticks(b, c.tb)?))
                    })
                    .transpose()
            })
            .collect::<Result<_>>()?;
        if range.is_some()
            && let Some((selected_pos, stream_index)) =
                selected.iter().copied().enumerate().find(|(_, i)| unsafe {
                    (*(*input.streams()[*i]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
                })
            && let Some((start, _)) = bounds[selected_pos]
            && coverage[selected_pos].reorder
        {
            // Locate the latest closed RAP at/before `start` (needed for open-GOP start
            // and for mid-GOP when demuxer seek alone lands on a non-IDR key).
            let tb = coverage[selected_pos].tb;
            let preroll = if tb.num > 0 && tb.den > 0 {
                let us = 3_000_000i128;
                let ticks = (us * i128::from(tb.den)) / (1_000_000i128 * i128::from(tb.num));
                i64::try_from(ticks).unwrap_or(i64::MAX / 4)
            } else {
                0
            };
            let target = start.saturating_sub(preroll.max(1));
            check(
                unsafe {
                    avformat_seek_file(input.0, stream_index as i32, i64::MIN, target, start, 0)
                },
                "seek before strict trim RAP scan",
            )?;
            let mut last_rap = None;
            let mut probe = Packet::new()?;
            while probe.read(&mut input)? {
                let (index, _) = packet_info(&probe, &input, options)?;
                if index != stream_index {
                    continue;
                }
                // SAFETY: Live packet/codecpar for RAP classification only.
                unsafe {
                    let p = &*probe.0;
                    let codec = &*(*input.streams()[index]).codecpar;
                    if p.pts == NOPTS {
                        return Err("missing packet PTS during RAP scan".into());
                    }
                    if p.pts > start {
                        break;
                    }
                    if closed_gop_rap(codec, p) {
                        last_rap = Some(p.pts);
                    }
                }
            }
            let origin_pts =
                last_rap.ok_or("requested start has no preceding closed-GOP RAP boundary")?;
            coverage[selected_pos].required_rap = Some(origin_pts);
            check(
                unsafe {
                    avformat_seek_file(input.0, stream_index as i32, i64::MIN, origin_pts, start, 0)
                },
                "seek to closed-GOP RAP before trim",
            )?;
        } else if range.is_some()
            && let Some((selected_pos, stream_index)) =
                selected.iter().copied().enumerate().find(|(_, i)| unsafe {
                    (*(*input.streams()[*i]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
                })
            && let Some((start, _)) = bounds[selected_pos]
        {
            check(
                unsafe {
                    avformat_seek_file(input.0, stream_index as i32, i64::MIN, start, start, 0)
                },
                "seek before strict trim range",
            )?;
        }
        let mut packet = Packet::new()?;
        while packet.read(&mut input)? {
            check_budget_with_bytes(options, stats.packets, stats.payload_bytes)?;
            let (index, size) = packet_info(&packet, &input, options)?;
            let Some(mapped) = selected.iter().position(|&i| i == index) else {
                continue;
            };
            let c = &mut coverage[mapped];
            if c.finished || c.decode_seam {
                continue;
            }
            // SAFETY: Packet and codec parameters are live; only packet timestamps are mutated.
            unsafe {
                let p = &mut *packet.0;
                let codec = &*(*input.streams()[index]).codecpar;
                if let Some((start, end)) = bounds[mapped] {
                    if p.pts == NOPTS || p.dts == NOPTS {
                        return Err("missing packet PTS/DTS".into());
                    }
                    if c.reorder {
                        // Closed-GOP H.264/HEVC and open-GOP with a preceding closed RAP:
                        // copy decode-order from the RAP at or before `start` through the first
                        // keyframe at/after `end` (excluded). Seek uses pre-roll so open-GOP
                        // start can land on the prior IDR/IRAP; mid-GOP keeps pre-roll packets
                        // with present_origin shift (negative PTS = edit-list window).
                        if c.origin.is_none() {
                            if p.pts > start {
                                return Err(
                                    "requested start has no preceding closed-GOP RAP boundary"
                                        .into(),
                                );
                            }
                            if !closed_gop_rap(codec, p) {
                                continue;
                            }
                            if let Some(required) = c.required_rap
                                && p.pts != required
                            {
                                continue;
                            }
                        } else if keyframe(p) && p.pts >= end {
                            // First keyframe at or after the presentation end closes the copy
                            // span (IDR/IRAP or open-GOP key). Exclude it from the output.
                            if p.pts == end {
                                c.saw_present_end = true;
                            }
                            c.end = Some(end);
                            c.finished = true;
                            continue;
                        }
                    } else {
                        let packet_end =
                            p.pts.checked_add(p.duration).ok_or("timestamp overflow")?;
                        if p.pts < start {
                            if packet_end > start {
                                return Err(
                                    "start cuts through a packet; exact stream copy impossible"
                                        .into(),
                                );
                            }
                            // Keep audio priming packets (negative PTS) when the presentation
                            // window starts at 0; otherwise skip fully-before-start packets.
                            if !(codec.codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO
                                && p.pts < 0
                                && start == 0)
                            {
                                continue;
                            }
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
                }
                let allow_reorder = c.reorder && bounds[mapped].is_some();
                strict_packet(p, codec, allow_reorder)?;
                if c.origin.is_none() {
                    independent(p, codec)?;
                    if let Some((start, _)) = bounds[mapped] {
                        if c.reorder {
                            // Decode starts at RAP <= start; presentation window is [start,end).
                            if p.pts > start {
                                return Err(
                                    "requested start has no preceding closed-GOP RAP boundary"
                                        .into(),
                                );
                            }
                            c.present_origin = Some(start);
                        } else if p.pts != start {
                            // Audio priming may begin before presentation start=0.
                            if !(codec.codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO
                                && p.pts < 0
                                && start == 0)
                            {
                                return Err("requested start has no exact packet boundary".into());
                            }
                            c.present_origin = Some(start);
                        } else {
                            c.present_origin = Some(start);
                        }
                    } else {
                        c.present_origin = Some(p.pts);
                    }
                    c.origin = Some(p.pts);
                }
                if let Some(present) = c.present_origin
                    && p.pts == present
                {
                    c.saw_present_start = true;
                }
                if let Some((_, end)) = bounds[mapped]
                    && p.pts == end
                {
                    c.saw_present_end = true;
                }
                // Post-roll inside the decode span: keep refs, hide presentation.
                if c.reorder
                    && let Some((_, end)) = bounds[mapped]
                    && p.pts >= end
                {
                    p.flags |= AV_PKT_FLAG_DISCARD as i32;
                }
                if c.reorder {
                    if let Some(previous) = c.dts_end
                        && previous != p.dts
                    {
                        return Err("packet gap/overlap rejected by strict editing".into());
                    }
                    c.dts_end = Some(
                        p.dts
                            .checked_add(p.duration)
                            .ok_or("packet DTS end overflow")?,
                    );
                    let presentation_end = p
                        .pts
                        .checked_add(p.duration)
                        .ok_or("packet PTS end overflow")?;
                    c.pts_end = Some(match c.pts_end {
                        Some(prev) => prev.max(presentation_end),
                        None => presentation_end,
                    });
                } else {
                    if let Some(previous) = c.end
                        && previous != p.pts
                    {
                        return Err("packet gap/overlap rejected by strict editing".into());
                    }
                    c.end = Some(p.pts.checked_add(p.duration).ok_or("packet end overflow")?);
                }
                let shift_origin = c
                    .present_origin
                    .ok_or("missing presentation origin for timeline shift")?;
                let shift = offsets[mapped]
                    .checked_sub(shift_origin)
                    .ok_or("timeline shift overflow")?;
                p.pts = p.pts.checked_add(shift).ok_or("PTS overflow")?;
                p.dts = p.dts.checked_add(shift).ok_or("DTS overflow")?;
            }
            output.write(&mut packet, mapped, c.tb)?;
            stats.packets += 1;
            stats.payload_bytes += size as u64;
        }
        if let Some((from_us, to_us)) = range {
            for &(stream_index, mapped, rate) in &decode_audio {
                let written = audio::mux_interval_pcm_from_path(
                    source,
                    &mut output,
                    stream_index,
                    mapped,
                    (from_us, to_us),
                    options.max_packet_bytes,
                )?;
                let sample_at = |time_us: i64| -> Result<i64> {
                    let scaled = i128::from(time_us) * i128::from(rate);
                    let samples = (scaled + 999_999) / 1_000_000;
                    i64::try_from(samples).map_err(|_| "audio interval overflow".into())
                };
                let start = sample_at(from_us)?;
                let end = sample_at(to_us)?;
                if i64::try_from(written).map_err(|_| "audio timeline overflow")?
                    != end.checked_sub(start).ok_or("audio duration overflow")?
                {
                    return Err("compressed-audio seam wrote unexpected sample count".into());
                }
                coverage[mapped].present_origin = Some(start);
                coverage[mapped].origin = Some(start);
                coverage[mapped].end = Some(end);
                coverage[mapped].tb = AVRational { num: 1, den: rate };
                coverage[mapped].saw_present_start = true;
                coverage[mapped].saw_present_end = true;
                stats.packets = stats.packets.saturating_add(1);
            }
        }
        let mut durations = Vec::new();
        for (i, c) in coverage.iter().enumerate() {
            if c.decode_seam {
                let first = c
                    .present_origin
                    .ok_or("selected stream has no packets in range")?;
                let end = c.end.ok_or("selected stream is empty")?;
                durations.push(end.checked_sub(first).ok_or("duration overflow")?);
                if i > 0
                    && (!same_time(
                        first,
                        c.tb,
                        coverage[0].present_origin.unwrap(),
                        coverage[0].tb,
                    ) || !same_time(durations[i], c.tb, durations[0], coverage[0].tb))
                {
                    return Err("audio/video starts or ends differ; strict concat/trim would create a gap or sync shift".into());
                }
                offsets[i] = offsets[i]
                    .checked_add(durations[i])
                    .ok_or("concat timeline overflow")?;
                continue;
            }
            let first = c
                .present_origin
                .ok_or("selected stream has no packets in range")?;
            if c.reorder && bounds[i].is_some() {
                if !c.saw_present_start {
                    return Err("requested start has no exact packet boundary".into());
                }
                if !c.saw_present_end {
                    return Err("requested end has no exact packet boundary".into());
                }
            }
            let end = if c.reorder {
                match (c.end, c.pts_end, bounds[i]) {
                    (Some(end), _, _) => end,
                    (None, Some(pts_end), Some((_, target))) if pts_end == target => target,
                    (None, Some(_), Some(_)) => {
                        return Err("requested end has no following keyframe boundary".into());
                    }
                    (None, Some(pts_end), None) => pts_end,
                    _ => return Err("selected stream is empty".into()),
                }
            } else {
                c.end.ok_or("selected stream is empty")?
            };
            if let Some((_, target)) = bounds[i]
                && end != target
            {
                return Err("requested end has no exact packet boundary".into());
            }
            durations.push(end.checked_sub(first).ok_or("duration overflow")?);
            if i > 0
                && (!same_time(
                    first,
                    c.tb,
                    coverage[0].present_origin.unwrap(),
                    coverage[0].tb,
                ) || !same_time(durations[i], c.tb, durations[0], coverage[0].tb))
            {
                return Err("audio/video starts or ends differ; strict concat/trim would create a gap or sync shift".into());
            }
            offsets[i] = offsets[i]
                .checked_add(durations[i])
                .ok_or("concat timeline overflow")?;
        }
        for (i, duration) in durations.iter().enumerate() {
            if coverage[i].reorder {
                output.set_stream_duration(i, *duration, coverage[i].tb)?;
            }
        }
    }
    emit_progress_done(options, stats.packets, stats.payload_bytes);
    output.finish()?;
    Ok(stats)
}

/// Closed-GOP H.264/HEVC stream-copy of one video into an existing muxer for `[from_us,to_us)`.
/// Uses a dedicated demuxer so the caller's primary decode position is undisturbed.
/// Same RAP/mid-GOP present_origin + DISCARD post-roll contract as `trim`.
pub(crate) fn mux_reordered_video_interval(
    source: &Path,
    output: &mut Output,
    stream_index: usize,
    mapped: usize,
    from_us: i64,
    to_us: i64,
    options: &CopyOptions,
) -> Result<u64> {
    if from_us < 0 || to_us <= from_us {
        return Err("reordered secondary interval requires 0 <= from < to".into());
    }
    let mut input = Input::open_fast(source)?;
    if stream_index >= input.streams().len() {
        return Err("secondary video stream index out of range".into());
    }
    // SAFETY: Index checked; codecpar owned by Input.
    let codec = unsafe { &*(*input.streams()[stream_index]).codecpar };
    if codec.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO {
        return Err("reordered secondary interval requires a video stream".into());
    }
    if codec.codec_id != AVCodecID_AV_CODEC_ID_H264 && codec.codec_id != AVCodecID_AV_CODEC_ID_HEVC
    {
        return Err("reordered secondary interval supports H.264 and HEVC only".into());
    }
    let tb = unsafe { (*input.streams()[stream_index]).time_base };
    let format_start_us = unsafe {
        if (*input.0).start_time == NOPTS {
            0
        } else {
            (*input.0).start_time
        }
    };
    let start = ticks(
        from_us
            .checked_add(format_start_us)
            .ok_or("start overflow")?,
        tb,
    )?;
    let end = ticks(
        to_us.checked_add(format_start_us).ok_or("end overflow")?,
        tb,
    )?;
    let preroll = if tb.num > 0 && tb.den > 0 {
        let us = 3_000_000i128;
        let ticks = (us * i128::from(tb.den)) / (1_000_000i128 * i128::from(tb.num));
        i64::try_from(ticks).unwrap_or(i64::MAX / 4)
    } else {
        0
    };
    let target = start.saturating_sub(preroll.max(1));
    check(
        unsafe { avformat_seek_file(input.0, stream_index as i32, i64::MIN, target, start, 0) },
        "seek before secondary RAP scan",
    )?;
    let mut last_rap = None;
    let mut probe = Packet::new()?;
    while probe.read(&mut input)? {
        let (index, _) = packet_info(&probe, &input, options)?;
        if index != stream_index {
            continue;
        }
        // SAFETY: Live packet/codecpar for RAP classification only.
        unsafe {
            let p = &*probe.0;
            let codec = &*(*input.streams()[index]).codecpar;
            if p.pts == NOPTS {
                return Err("missing packet PTS during secondary RAP scan".into());
            }
            if p.pts > start {
                break;
            }
            if closed_gop_rap(codec, p) {
                last_rap = Some(p.pts);
            }
        }
    }
    let required_rap =
        last_rap.ok_or("secondary interval has no preceding closed-GOP RAP boundary")?;
    check(
        unsafe {
            avformat_seek_file(
                input.0,
                stream_index as i32,
                i64::MIN,
                required_rap,
                start,
                0,
            )
        },
        "seek to secondary closed-GOP RAP",
    )?;
    let mut packet = Packet::new()?;
    let mut origin = None;
    let mut saw_present_start = false;
    let mut saw_present_end = false;
    let mut finished = false;
    let mut copied = 0u64;
    while packet.read(&mut input)? {
        check_budget(options, copied)?;
        let (index, size) = packet_info(&packet, &input, options)?;
        if index != stream_index {
            continue;
        }
        if finished {
            break;
        }
        // SAFETY: Live packet/codec; timestamps mutated for present_origin rebase.
        unsafe {
            let p = &mut *packet.0;
            let codec = &*(*input.streams()[index]).codecpar;
            if p.pts == NOPTS || p.dts == NOPTS || p.duration <= 0 {
                return Err(
                    "secondary reordered interval requires PTS, DTS and positive duration".into(),
                );
            }
            if origin.is_none() {
                if p.pts > start {
                    return Err(
                        "secondary interval has no preceding closed-GOP RAP boundary".into(),
                    );
                }
                if !closed_gop_rap(codec, p) {
                    continue;
                }
                if p.pts != required_rap {
                    continue;
                }
                origin = Some(p.pts);
            } else if keyframe(p) && p.pts >= end {
                if p.pts == end {
                    saw_present_end = true;
                }
                finished = true;
                continue;
            }
            if p.pts == start {
                saw_present_start = true;
            }
            if p.pts == end {
                saw_present_end = true;
            }
            if p.pts >= end {
                p.flags |= AV_PKT_FLAG_DISCARD as i32;
            }
            let shift = start;
            p.pts = p.pts.checked_sub(shift).ok_or("PTS overflow")?;
            p.dts = p.dts.checked_sub(shift).ok_or("DTS overflow")?;
            let _ = size;
        }
        output.write(&mut packet, mapped, tb)?;
        copied += 1;
    }
    if !saw_present_start {
        return Err("secondary interval start has no exact packet boundary".into());
    }
    if !saw_present_end {
        return Err("secondary interval end has no exact packet boundary".into());
    }
    if copied == 0 {
        return Err("secondary reordered interval copied no packets".into());
    }
    let duration = end.checked_sub(start).ok_or("duration overflow")?;
    output.set_stream_duration(mapped, duration, tb)?;
    Ok(copied)
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
        // HEVC IDR_N_LP (type 20) after start code.
        assert!(hevc_irap(&[0, 0, 0, 1, 20 << 1, 1], &[]));
        assert!(!hevc_irap(&[0, 0, 0, 1, 1 << 1, 1], &[]));
    }
}
