//! Exact packed-PCM packet slicing. Payload ownership stays with AVPacket.buf.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntervalAudio {
    /// Packet-trim path for packed PCM without padding.
    Pcm,
    /// Decode to PCM for AAC/MP3/FLAC during lossless intervals.
    Decode,
}

pub(super) fn classify_interval_audio(parameters: &AVCodecParameters) -> Result<IntervalAudio> {
    if parameters.codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO {
        return Err("lossless interval non-video stream must be audio".into());
    }
    if is_packed_pcm(parameters) {
        if parameters.initial_padding != 0 || parameters.trailing_padding != 0 {
            return Err(
                "lossless interval audio requires packed PCM without padding; compressed audio trimming is not implemented"
                    .into(),
            );
        }
        return Ok(IntervalAudio::Pcm);
    }
    if [
        AVCodecID_AV_CODEC_ID_AAC,
        AVCodecID_AV_CODEC_ID_MP3,
        AVCodecID_AV_CODEC_ID_FLAC,
    ]
    .contains(&parameters.codec_id)
    {
        return Ok(IntervalAudio::Decode);
    }
    Err(
        "lossless interval audio requires packed PCM or AAC/MP3/FLAC; other codecs are not implemented"
            .into(),
    )
}

fn packed_pcm_bytes(codec: AVCodecID) -> Option<usize> {
    Some(match codec {
        AVCodecID_AV_CODEC_ID_PCM_U8 | AVCodecID_AV_CODEC_ID_PCM_S8 => 1,
        AVCodecID_AV_CODEC_ID_PCM_S16LE | AVCodecID_AV_CODEC_ID_PCM_S16BE => 2,
        AVCodecID_AV_CODEC_ID_PCM_S24LE | AVCodecID_AV_CODEC_ID_PCM_S24BE => 3,
        AVCodecID_AV_CODEC_ID_PCM_S32LE | AVCodecID_AV_CODEC_ID_PCM_S32BE
        | AVCodecID_AV_CODEC_ID_PCM_F32LE | AVCodecID_AV_CODEC_ID_PCM_F32BE => 4,
        AVCodecID_AV_CODEC_ID_PCM_F64LE | AVCodecID_AV_CODEC_ID_PCM_F64BE => 8,
        _ => return None,
    })
}
fn is_packed_pcm(parameters: &AVCodecParameters) -> bool {
    packed_pcm_bytes(parameters.codec_id).is_some()
}

pub(super) fn validate(parameters: &AVCodecParameters) -> Result<usize> {
    if !is_packed_pcm(parameters)
        || parameters.sample_rate <= 0
        || parameters.ch_layout.nb_channels <= 0
        || parameters.initial_padding != 0
        || parameters.trailing_padding != 0
    {
        return Err("lossless interval audio requires packed PCM without padding; compressed audio trimming is not implemented".into());
    }
    let bytes = packed_pcm_bytes(parameters.codec_id).ok_or("invalid PCM sample size")?;
    bytes
        .checked_mul(parameters.ch_layout.nb_channels as usize)
        .filter(|&v| v != 0)
        .ok_or_else(|| "invalid PCM frame size".into())
}
fn exact(numerator: i128, denominator: i128) -> Result<i64> {
    if denominator <= 0 || numerator % denominator != 0 {
        return Err("PCM time boundary is not exactly representable".into());
    }
    i64::try_from(numerator / denominator).map_err(|_| "PCM timestamp overflow".into())
}
/// Return retained sample frames (all channels), adjusting only the packet view.
pub(super) fn trim(
    packet: &mut Packet,
    input: &Input,
    index: usize,
    from: i64,
    to: i64,
) -> Result<u64> {
    // SAFETY: Packet and selected input stream are live and exclusively owned where
    // modified. The offset and retained size are checked against the original payload.
    // AVPacket.buf is untouched, so unref still releases the original allocation.
    unsafe {
        let stream = &*input.streams()[index];
        let parameters = &*stream.codecpar;
        let frame_bytes = validate(parameters)?;
        let p = &mut *packet.0;
        if p.pts == NOPTS
            || p.dts != p.pts
            || p.duration <= 0
            || p.side_data_elems != 0
            || p.size <= 0
            || p.data.is_null()
            || !(p.size as usize).is_multiple_of(frame_bytes)
        {
            return Err(
                "PCM slicing requires timestamped whole sample frames without side data".into(),
            );
        }
        let tb = stream.time_base;
        if tb.num <= 0 || tb.den <= 0 {
            return Err("invalid PCM time base".into());
        }
        let rate = i128::from(parameters.sample_rate);
        let position = exact(
            i128::from(p.pts) * i128::from(tb.num) * rate,
            i128::from(tb.den),
        )?;
        let count = (p.size as usize / frame_bytes) as i64;
        if exact(
            i128::from(p.duration) * i128::from(tb.num) * rate,
            i128::from(tb.den),
        )? != count
        {
            return Err("PCM packet duration disagrees with sample payload".into());
        }
        let origin = if (*input.0).start_time == NOPTS {
            0
        } else {
            (*input.0).start_time
        };
        let start = exact(
            i128::from(origin.checked_add(from).ok_or("PCM interval overflow")?) * rate,
            1_000_000,
        )?;
        let end = exact(
            i128::from(origin.checked_add(to).ok_or("PCM interval overflow")?) * rate,
            1_000_000,
        )?;
        let left = position.max(start);
        let right = position
            .checked_add(count)
            .ok_or("PCM packet range overflow")?
            .min(end);
        if left >= right {
            return Ok(0);
        }
        let retained = right - left;
        let pts = exact(
            i128::from(left - start) * i128::from(tb.den),
            rate * i128::from(tb.num),
        )?;
        let duration = exact(
            i128::from(retained) * i128::from(tb.den),
            rate * i128::from(tb.num),
        )?;
        let offset = (left - position) as usize * frame_bytes;
        p.data = p.data.add(offset);
        p.size = i32::try_from(retained as usize * frame_bytes)
            .map_err(|_| "PCM packet size overflow")?;
        p.pts = pts;
        p.dts = pts;
        p.duration = duration;
        Ok(retained as u64)
    }
}

pub use fvid_media_info::PcmTrimStats;

/// Cut selected packed PCM streams without decoding or allocating a new payload.
pub fn trim_pcm(
    source: &Path,
    destination: &Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<PcmTrimStats> {
    if from < 0 || to <= from {
        return Err("PCM interval requires 0 <= from < to".into());
    }
    if crate::owned_wave_remux::supports(source, destination, options) {
        return crate::owned_wave_remux::trim_pcm(source, destination, from, to, options);
    }
    let mut input = Input::open(source)?;
    let selected = selection(&input, options)?;
    for &index in &selected {
        // SAFETY: Stream indices were checked against the live input table.
        validate(unsafe { &*(*input.streams()[index]).codecpar })?;
    }
    lossless::retime_chapters(&mut input, from, to)?;
    let mut output = Output::new(destination, &input, &selected)?;
    output.strict_timing = true;
    let mut packet = Packet::new()?;
    let mut stats = PcmTrimStats {
        packets: 0,
        sample_frames: 0,
        payload_bytes: 0,
        fvid_payload_copies: 0,
    };
    while packet.read(&mut input)? {
        let (index, _) = packet_info(&packet, &input, options)?;
        let Some(mapped) = selected.iter().position(|&i| i == index) else {
            continue;
        };
        let samples = trim(&mut packet, &input, index, from, to)?;
        if samples == 0 {
            continue;
        }
        // SAFETY: Packet is live and trim checked its retained nonnegative size.
        stats.payload_bytes += unsafe { (*packet.0).size as u64 };
        let tb = unsafe { (*input.streams()[index]).time_base };
        output.write(&mut packet, mapped, tb)?;
        stats.sample_frames += samples;
        stats.packets += 1;
    }
    if stats.sample_frames == 0 {
        return Err("no PCM samples in selected interval".into());
    }
    output.finish()?;
    Ok(stats)
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[test]
    fn packed_codec_sizes_cover_both_endian_orders_and_reject_compression() {
        for (codecs, bytes) in [
            ([AVCodecID_AV_CODEC_ID_PCM_U8, AVCodecID_AV_CODEC_ID_PCM_S8], 1),
            ([AVCodecID_AV_CODEC_ID_PCM_S16LE, AVCodecID_AV_CODEC_ID_PCM_S16BE], 2),
            ([AVCodecID_AV_CODEC_ID_PCM_S24LE, AVCodecID_AV_CODEC_ID_PCM_S24BE], 3),
            ([AVCodecID_AV_CODEC_ID_PCM_S32LE, AVCodecID_AV_CODEC_ID_PCM_S32BE], 4),
            ([AVCodecID_AV_CODEC_ID_PCM_F32LE, AVCodecID_AV_CODEC_ID_PCM_F32BE], 4),
            ([AVCodecID_AV_CODEC_ID_PCM_F64LE, AVCodecID_AV_CODEC_ID_PCM_F64BE], 8),
        ] {
            for codec in codecs {
                // SAFETY: Metadata has no owned pointers; validate only reads scalar fields.
                let mut parameters: AVCodecParameters = unsafe { std::mem::zeroed() };
                parameters.codec_id = codec;
                parameters.sample_rate = 48000;
                parameters.ch_layout.nb_channels = 6;
                assert_eq!(validate(&parameters).unwrap(), bytes * 6);
                parameters.initial_padding = 1;
                assert!(validate(&parameters).is_err());
            }
        }
        assert_eq!(packed_pcm_bytes(AVCodecID_AV_CODEC_ID_AAC), None);
        assert_eq!(packed_pcm_bytes(AVCodecID_AV_CODEC_ID_NONE), None);
    }
}
