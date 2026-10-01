//! Decode text subtitles and re-encode to another text codec (no burn-in).
use super::*;
use lossless::{Codec, LosslessStats, LosslessTransform, OverlaySpec, Parameters};
use std::path::Path;

pub use fvid_media_info::{SubtitleCodec, SubtitleConvertOptions};

pub use fvid_media_info::NativeSubtitleStats as SubtitleConvertStats;

struct Subtitle(AVSubtitle);
impl Default for Subtitle {
    fn default() -> Self {
        Self(unsafe { std::mem::zeroed() })
    }
}
impl Drop for Subtitle {
    fn drop(&mut self) {
        unsafe {
            avsubtitle_free(&mut self.0);
        }
    }
}

/// Convert a single text subtitle stream (qualified: SubRip → ASS in Matroska).
pub fn convert_subtitles(
    source: &Path,
    destination: &Path,
    options: &SubtitleConvertOptions,
) -> Result<SubtitleConvertStats> {
    if destination.extension().and_then(|v| v.to_str()) != Some("mkv") {
        return Err("convert-subtitles requires Matroska (.mkv) output".into());
    }
    let mut input = Input::open_fast(source)?;
    if unsafe { (*input.0).nb_chapters } != 0 {
        return Err("convert-subtitles with chapters is not qualified".into());
    }
    let copy = CopyOptions {
        streams: options.streams.clone(),
        ..Default::default()
    };
    let selected = if copy.streams.is_empty() {
        let index = input
            .streams()
            .iter()
            .position(|&s| unsafe {
                (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_SUBTITLE
            })
            .ok_or("input has no subtitle stream")?;
        vec![index]
    } else {
        selection(&input, &copy)?
    };
    if selected.len() != 1 {
        return Err(
            "convert-subtitles requires exactly one selected subtitle stream; use --streams".into(),
        );
    }
    let index = selected[0];
    // SAFETY: Input owns live streams/codecpar; RAII owns allocated codecs.
    let (decoder, encoder, parameters, time_base) = unsafe {
        let stream = &*input.streams()[index];
        let parameters_in = &*stream.codecpar;
        if parameters_in.codec_type != AVMediaType_AVMEDIA_TYPE_SUBTITLE {
            return Err("selected stream is not a subtitle".into());
        }
        if parameters_in.codec_id != AVCodecID_AV_CODEC_ID_SUBRIP
            && parameters_in.codec_id != AVCodecID_AV_CODEC_ID_SRT
        {
            return Err("convert-subtitles v1 accepts SubRip/SRT input only".into());
        }
        let dec = avcodec_find_decoder(parameters_in.codec_id);
        if dec.is_null() {
            return Err("subtitle decoder unavailable".into());
        }
        let decoder = Codec(avcodec_alloc_context3(dec));
        if decoder.0.is_null() {
            return Err("subtitle decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(decoder.0, stream.codecpar),
            "configure subtitle decoder",
        )?;
        (*decoder.0).pkt_timebase = stream.time_base;
        check(
            avcodec_open2(decoder.0, dec, ptr::null_mut()),
            "open subtitle decoder",
        )?;

        let enc_name = cstring(options.codec.encoder_name())?;
        let enc = avcodec_find_encoder_by_name(enc_name.as_ptr());
        if enc.is_null() {
            return Err(format!(
                "subtitle encoder unavailable: {}",
                options.codec.encoder_name()
            ));
        }
        let encoder = Codec(avcodec_alloc_context3(enc));
        if encoder.0.is_null() {
            return Err("subtitle encoder allocation failed".into());
        }
        (*encoder.0).time_base = AVRational { num: 1, den: 1000 };
        check(
            avcodec_open2(encoder.0, enc, ptr::null_mut()),
            "open subtitle encoder",
        )?;
        let parameters = Parameters(avcodec_parameters_alloc());
        if parameters.0.is_null() {
            return Err("subtitle parameter allocation failed".into());
        }
        check(
            avcodec_parameters_from_context(parameters.0, encoder.0),
            "export subtitle encoder parameters",
        )?;
        let time_base = (*encoder.0).time_base;
        (decoder, encoder, parameters, time_base)
    };

    let mut output = Output::with_video_direct(
        destination,
        &input,
        &selected,
        Some((index, parameters.0.cast_const(), time_base)),
    )?
    .without_interleave();

    let mut packet = Packet::new()?;
    let mut encoded = Packet::new()?;
    let mut subtitle = Subtitle::default();
    let mut cues = 0u64;
    let mut packets_in = 0u64;
    let mut packets_out = 0u64;
    let mut payload_bytes = 0u64;
    // Large enough for typical ASS dialogue packets; encoder reports needed size on failure.
    let mut buffer = vec![0u8; 1 << 20];

    while packet.read(&mut input)? {
        if unsafe { (*packet.0).stream_index } != index as i32 {
            continue;
        }
        packets_in += 1;
        let mut got = 0i32;
        // SAFETY: Decoder, subtitle, and packet are live for the call.
        check(
            unsafe { avcodec_decode_subtitle2(decoder.0, &mut subtitle.0, &mut got, packet.0) },
            "decode subtitle cue",
        )?;
        if got == 0 {
            continue;
        }
        unsafe {
            for rect_index in 0..subtitle.0.num_rects {
                let rect = *subtitle.0.rects.add(rect_index as usize);
                if rect.is_null() {
                    continue;
                }
                let kind = (*rect).type_;
                if kind != AVSubtitleType_SUBTITLE_TEXT && kind != AVSubtitleType_SUBTITLE_ASS {
                    avsubtitle_free(&mut subtitle.0);
                    return Err("bitmap subtitles are not qualified for convert-subtitles".into());
                }
            }
        }
        let size = unsafe {
            avcodec_encode_subtitle(
                encoder.0,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                &subtitle.0,
            )
        };
        if size < 0 {
            unsafe { avsubtitle_free(&mut subtitle.0) };
            check(size, "encode subtitle cue")?;
            unreachable!();
        }
        if size == 0 {
            unsafe { avsubtitle_free(&mut subtitle.0) };
            continue;
        }
        unsafe {
            av_packet_unref(encoded.0);
            check(av_new_packet(encoded.0, size), "allocate subtitle packet")?;
            ptr::copy_nonoverlapping(buffer.as_ptr(), (*encoded.0).data, size as usize);
            let src_tb = (*input.streams()[index]).time_base;
            let pts = if (*packet.0).pts != NOPTS {
                (*packet.0).pts
            } else if subtitle.0.pts != NOPTS {
                // AVSubtitle.pts is in AV_TIME_BASE.
                super::rescale_owned(
                    subtitle.0.pts,
                    AVRational {
                        num: 1,
                        den: AV_TIME_BASE as i32,
                    },
                    src_tb,
                )?
            } else {
                avsubtitle_free(&mut subtitle.0);
                return Err("subtitle cue requires timestamps".into());
            };
            let duration = if (*packet.0).duration > 0 {
                (*packet.0).duration
            } else {
                let start = i64::from(subtitle.0.start_display_time);
                let end = i64::from(subtitle.0.end_display_time);
                if end > start {
                    super::rescale_owned(end - start, AVRational { num: 1, den: 1000 }, src_tb)?
                } else {
                    0
                }
            };
            // Encode into encoder time base for the mux override.
            (*encoded.0).pts = super::rescale_owned(pts, src_tb, time_base)?;
            (*encoded.0).dts = (*encoded.0).pts;
            (*encoded.0).duration = super::rescale_owned(duration, src_tb, time_base)?;
            (*encoded.0).flags |= AV_PKT_FLAG_KEY as i32;
        }
        payload_bytes += size as u64;
        output.write(&mut encoded, 0, time_base)?;
        packets_out += 1;
        cues += 1;
        unsafe { avsubtitle_free(&mut subtitle.0) };
    }
    if cues == 0 {
        return Err("no subtitle cues were converted".into());
    }
    output.finish()?;
    Ok(SubtitleConvertStats {
        backend: "native libavcodec subtitle",
        encoder: options.codec.encoder_name().into(),
        cues,
        packets_in,
        packets_out,
        payload_bytes,
    })
}

/// Burn an external `.srt`/text subtitle file onto video via libavfilter `subtitles=`.
/// Video-only FFV1/Matroska output; fair-pairs FFmpeg `-vf subtitles=FILE -c:v ffv1 -an`.
pub fn burn_subtitles(
    source: &Path,
    destination: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    let input = Input::open(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&stream| unsafe {
            (*(*stream).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .ok_or("burn-subtitles requires a video stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![video];
    } else if selected.streams.as_slice() != [video] {
        return Err("burn-subtitles v1 selects only the first video stream".into());
    }
    drop(input);
    lossless::transcode_lossless(
        source,
        destination,
        LosslessTransform {
            burn_subs: Some(subs.to_path_buf()),
            ..LosslessTransform::default()
        },
        &selected,
    )
}

/// Composite an overlay video onto the main video via libavfilter `movie=` + `overlay=`.
/// Video-only FFV1/Matroska; fair-pairs FFmpeg `-vf movie=FILE[ov];[in][ov]overlay=x:y`.
pub fn overlay_video(
    source: &Path,
    overlay: &Path,
    destination: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    let input = Input::open(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&stream| unsafe {
            (*(*stream).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .ok_or("overlay requires a main video stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![video];
    } else if selected.streams.as_slice() != [video] {
        return Err("overlay v1 selects only the first video stream".into());
    }
    drop(input);
    if !overlay.is_file() {
        return Err("overlay video file not found".into());
    }
    lossless::transcode_lossless(
        source,
        destination,
        LosslessTransform {
            overlay: Some(OverlaySpec {
                path: overlay.to_path_buf(),
                x,
                y,
            }),
            ..LosslessTransform::default()
        },
        &selected,
    )
}
