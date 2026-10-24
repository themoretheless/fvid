//! Read-only operation plans for the owned WAVE, ADTS and MP4 audio writer.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeTransform, MediaPlan, PlanStep, PlanStream};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

pub(crate) fn matroska_descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<Option<(usize, u32, u16, u32, String, String)>> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 4];
    if file.read_exact(&mut prefix).is_err() || prefix != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(None);
    }
    let reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let selected = options.streams.first().copied();
    let index = crate::owned_matroska_audio::matroska_audio_index(&reader, selected)
        .map_err(|e| e.to_string())?;
    let source_codec = reader.tracks[index].codec.clone();
    drop(reader);
    let (rate, channels, mask, codec, precision) = match source_codec.as_str() {
        "A_FLAC" => {
            let (rate,channels)=crate::owned_matroska_flac_export::geometry(source,options)?;
            (rate,channels,crate::owned_matroska_flac_export::channel_mask(source,options)?,"flac".into(),"float32".into())
        }
        "A_AAC" => {
            let (rate, channels, mask) =
                crate::owned_matroska_aac_export::geometry(source, options)?;
            (rate, channels, mask, "aac".into(), "float32".into())
        }
        "A_OPUS" => {
            let (rate, channels, mask) =
                crate::owned_matroska_opus_export::geometry_with_mask(source, options)?;
            (rate, channels, mask, "opus".into(), "float32".into())
        }
        "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3" => {
            let (rate, channels) = crate::owned_matroska_mp3_export::geometry(source, options)?;
            (rate, channels, crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
             match source_codec.as_str() { "A_MPEG/L1" => "mp1", "A_MPEG/L2" => "mp2", _=>"mp3" }.into(), "float32".into())
        }
        "A_ALAC" => {
            let (rate, channels,mask) = crate::owned_matroska_alac_export::geometry_with_mask(source, options)?;
            (
                rate,
                channels,
                mask,
                "alac".into(),
                "float32".into(),
            )
        }
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
            let (rate, channels) = crate::owned_matroska_pcm_export::geometry(source, options)?;
            let mask = if channels <= 2 {
                crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
            } else {
                0
            };
            (
                rate,
                channels,
                mask,
                source_codec.clone(),
                "float64".into(),
            )
        }
        _ => return Err("selected Matroska audio codec is not owned by the export path".into()),
    };
    Ok(Some((index, rate, channels, mask, codec, precision)))
}
/// Admit only plans accepted by the owned metadata preflight.
/// This does not decode packets, create a destination, or emit progress.
pub(crate) fn supports(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match plan_decode_audio(source, transform, options) {
        Ok(_) => true,
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
pub(crate) struct AudioDescriptor {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    mask: u32,
    codec: String,
    precision: String,
    stream_index: usize,
    adts: bool,
    mp4: bool,
    is_matroska: bool,
    raw_encoded_audio: bool,
    ogg: bool,
    g711_wave: bool,
}

/// Read-only audio source geometry (`rate`, `channels`) for native loudness and planning.
pub fn source_geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    let desc = source_descriptor(source, options)?;
    Ok((desc.rate, desc.channels))
}

/// Read-only audio source geometry and speaker mask (`rate`, `channels`, `mask`).
pub fn source_geometry_with_mask(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let desc = source_descriptor(source, options)?;
    Ok((desc.rate, desc.channels, desc.mask))
}

pub(crate) fn source_descriptor(source: &Path, options: &CopyOptions) -> Result<AudioDescriptor> {
    source_descriptor_for_decode(source, options, None)
}
fn source_descriptor_for_decode(source: &Path, options: &CopyOptions,
    transform: Option<&AudioDecodeTransform>) -> Result<AudioDescriptor> {
    let adts = crate::owned_adts_export::recognizes(source)?;
    let raw_flac = !adts && crate::owned_raw_flac::recognizes(source)?;
    let raw_encoded_audio = raw_flac || (!adts && crate::owned_raw_mp3::recognizes(source)?);
    let mp4 = crate::owned_mp4_audio_export::recognizes(source, options)?;
    let matroska = matroska_descriptor(source, options)?;
    let is_matroska = matroska.is_some();
    let ogg = crate::owned_ogg_opus_export::recognizes(source)?;
    let g711_wave = crate::owned_g711_wave_export::recognizes(source, options)?;
    let mut stream_index = 0;
    let (rate, channels, mask, codec, precision) = if g711_wave {
        let (rate, channels, mask, codec, _) =
            crate::owned_g711_wave_export::descriptor(source, options)?;
        (rate, channels, mask, codec.to_owned(), "float32".to_owned())
    } else if ogg {
        let (index, rate, channels, mask, codec) =
            crate::owned_ogg_audio_export::descriptor(source, options)?;
        stream_index = index;
        (rate, channels, mask, codec.to_owned(), "float32".to_owned())
    } else if mp4 {
        let (index, rate, channels, mask, codec) =
            crate::owned_mp4_audio_export::descriptor(source, options)?;
        stream_index = index;
        let precision = if codec.starts_with("pcm_") {
            "float64"
        } else {
            "float32"
        };
        (rate, channels, mask, codec, precision.to_owned())
    } else if let Some((index, rate, channels, mask, codec, precision)) = matroska {
        stream_index = index;
        (rate, channels, mask, codec, precision)
    } else if raw_flac {
        let (rate,channels,mask)=crate::owned_raw_flac::descriptor(source,options)?;
        (rate,channels,mask,"flac".to_owned(),"float32".to_owned())
    } else if raw_encoded_audio {
        let (rate, channels, raw_codec) = crate::owned_raw_mp3::descriptor(source, options)?;
        (rate, channels, crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
         raw_codec.to_owned(), "float32".to_owned())
    } else if adts {
        if !crate::owned_audio_export::simple_options(options) {
            return Err("ADTS has only stream 0".into());
        }
        let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            options.max_packet_bytes,
        )
        .map_err(|e| e.to_string())?;
        let config = reader.configuration();
        let prefix = transform.and_then(|transform|
            crate::owned_adts_export::decoded_prefix(*transform, config.sample_rate));
        drop(reader);
        let (rate, channels, mask) = crate::owned_aac::stream::adts_prefix_info(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?), prefix, options)
            .map_err(|e| e.to_string())?;
        (
            rate,
            channels,
            mask,
            "aac".to_owned(),
            "float32".to_owned(),
        )
    } else {
        if !crate::owned_audio_export::simple_options(options) {
            return Err("WAVE has only stream 0".into());
        }
        let mut file = File::open(source).map_err(|e| e.to_string())?;
        let info = crate::owned_wave_inspect::inspect(&mut file, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
        (
            info.sample_rate,
            info.channels,
            info.channel_mask,
            info.codec(),
            if info.float {
                format!("float{}", info.bits_per_sample)
            } else {
                format!(
                    "integer{}",
                    if info.bits_per_sample == 24 {
                        32
                    } else {
                        info.bits_per_sample
                    }
                )
            },
        )
    };
    Ok(AudioDescriptor {
        rate, channels, mask, codec, precision, stream_index,
        adts, mp4, is_matroska, raw_encoded_audio, ogg, g711_wave,
    })
}

pub fn plan_decode_audio(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    crate::owned_audio_export::validate_request(*transform, options)?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let AudioDescriptor {
        rate, channels, mask, codec, precision, stream_index,
        adts, mp4, is_matroska, raw_encoded_audio, ogg, g711_wave,
    } = source_descriptor_for_decode(source, options, Some(transform))?;
    let output_channels = transform.channels.unwrap_or(i32::from(channels));
    let aac_fold=codec=="aac"&&output_channels!=i32::from(channels)&&matches!(output_channels,1|2)&&crate::owned_aac_export_routing::router(source,rate,channels,options)?.is_some();
    if output_channels != i32::from(channels) && !aac_fold {
        if !((channels <= 8 && matches!(output_channels, 1 | 2))
            || (channels <= 2 && (1..=8).contains(&output_channels)))
        {
            return Err("unsupported channel conversion".into());
        }
        if !(channels <= 2 && mask == 0)
            && !(mask != 0 && crate::owned_pcm_channels::rematrix_layout_supported(channels, mask))
        {
            return Err("unsupported speaker layout".into());
        }
    }
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: if mp4 {
            format!("owned MP4 {codec} decoder and presentation scheduler; preserve silence, repeated edits and AAC preroll; private {precision} WAVE disk spool")
        } else if is_matroska {
            format!("owned Matroska {codec} decoder and presentation scheduler; preserve CodecDelay, signed padding and gaps; private {precision} WAVE disk spool")
        } else if ogg {
            format!("owned Ogg {codec} decoder and presentation scheduler; preserve chained links, pre-skip and rational rate changes; private {precision} WAVE disk spool")
        } else if g711_wave {
            format!("owned WAVE {codec} decoder; preserve speaker mask {mask:#x}; private {precision} WAVE disk spool")
        } else if raw_encoded_audio {
            format!("owned raw {} decoder; validated sample timeline and encoded packet limits; private float32 WAVE disk spool", codec.to_uppercase())
        } else if adts {
            "owned AAC-LC/HE-AAC decoder; negotiate implicit SBR output clock; retain ADTS priming and preroll; private float32 WAVE disk spool"
                .into()
        } else {
            format!("owned WAVE reader; retain {precision} precision and speaker mask {mask:#x}")
        },
    }];
    if output_channels != i32::from(channels) {
        steps.push(PlanStep {
            action: "rematrix".into(),
            detail: format!("owned channel conversion {channels} → {output_channels}"),
        });
    }
    if let Some(gain) = transform.volume.filter(|&g| g != 1.0) {
        steps.push(PlanStep {
            action: "volume".into(),
            detail: format!("owned linear gain {gain} after rematrix and before resampling"),
        });
    }
    if let Some(output_rate) = transform.sample_rate.filter(|&r| r != rate as i32) {
        steps.push(PlanStep {
            action: "resample".into(),
            detail: format!("owned windowed-sinc resampler {rate} → {output_rate} Hz"),
        });
    }
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep{action:"trim".into(),detail:format!("output sample window [{from},{to}) µs; ceil boundaries after resampling; compressed prefix includes required preroll and resampler lookahead")});
    }
    if !options.metadata_set.is_empty() || !options.metadata_delete.is_empty() {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply owned RIFF INFO container tag edits to final WAVE".into(),
        });
    }
    steps.push(PlanStep{action:"write".into(),detail:format!("owned .wav writer; preserve {precision} precision; publish without overwriting; cleanup on failure")});
    Ok(MediaPlan{command:"decode-audio".into(),input:source.into(),inputs:vec![source.into()],streams:vec![PlanStream{index:stream_index,media_type:"audio".into(),codec,disposition:"decode".into()}],steps,graph:None,notes:vec!["backend: owned fvid-media; no external demuxer, codec, resampler or muxer".into(),if adts {"read-only ADTS prefix negotiation checks the output clock and decoder admission without emitting progress; packet payloads and publication are revalidated during execution".into()} else {"read-only metadata preflight: packet tools, payload validity, DSP allocation admission and publication are checked during execution".into()},if adts || mp4 || is_matroska || raw_encoded_audio || ogg || g711_wave {"packet byte/count limits apply to selected encoded audio, including preroll; internal PCM blocks are excluded".into()} else {"packet limits apply to frame-aligned WAVE I/O blocks of at most 4096 sample frames".into()}]})
}
