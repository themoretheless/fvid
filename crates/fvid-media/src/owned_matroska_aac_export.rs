//! Matroska AAC file export via owned presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn decode_options(options: &CopyOptions) -> CopyOptions {
    let mut options = options.clone();
    options.metadata_set.clear();
    options.metadata_delete.clear();
    options.stream_metadata_set.clear();
    options.stream_metadata_delete.clear();
    options
}
use crate::owned_adts_export::decoded_prefix as prefix;
#[cfg(test)]
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let reader = match crate::owned_matroska_audio::open_aac_reader(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        options,
        options.max_packet_bytes,
    ) {
        Ok(reader) => reader,
        Err(error) => return error.to_string().starts_with("controlled memory budget exceeded:"),
    };
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return false,
    };
    let Ok(index) = crate::owned_matroska_audio::matroska_audio_index(&reader, selected) else {
        return false;
    };
    supports_from_reader(source, reader, index, transform, options)
}
#[cfg(test)]
pub(crate) fn supports_from_reader<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, mask, _, _, mut reader) =
        match geometry_and_reader_for_decode(source, reader, index, Some(transform), options) {
            Ok(geometry) => geometry,
            Err(error) => return error.starts_with("controlled memory budget exceeded:"),
        };
    if let Some(target) = transform.channels {
        if target != i32::from(channels) {
            let aac_fold = if matches!(target, 1 | 2) {
                match crate::owned_aac_export_routing::router_from_matroska_reader(
                    &mut reader,
                    index,
                    rate,
                    channels,
                    Some(transform),
                    options,
                ) {
                    Ok(router) => router.is_some(),
                    Err(error) => return error.starts_with("controlled memory budget exceeded:"),
                }
            } else {
                false
            };
            let rematrix_ok = ((channels <= 2 && (1..=8).contains(&target))
                || (channels <= 8 && matches!(target, 1 | 2)))
                && crate::owned_pcm_channels::standard_mask(channels)
                    .is_some_and(|standard| standard == u64::from(mask));
            if !aac_fold && !rematrix_ok {
                return false;
            }
        }
    }
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_audio::decode_matroska_audio_reader_pcm(
        reader,
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok()
        || result
            .err()
            .is_some_and(|e| e.to_string().starts_with("controlled memory budget exceeded:"))
}

pub(crate) fn apply_from_reader<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let (rate, channels, mask, frames, info, mut reader) =
        geometry_and_reader_for_decode(source, reader, index, Some(transform), options)?;
    let router = if transform
        .channels
        .is_some_and(|output| output != i32::from(channels) && matches!(output, 1 | 2))
    {
        crate::owned_aac_export_routing::router_from_matroska_reader(
            &mut reader,
            index,
            rate,
            channels,
            Some(transform),
            options,
        )?
    } else {
        None
    };
    let aac_fold = router.is_some();
    crate::owned_adts_export::check_spool_wave_export_budget(
        rate,
        channels,
        32,
        frames.max(1024),
        info.len(),
        aac_fold,
        transform,
        options,
    )?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| {
            crate::owned_matroska_audio::decode_matroska_audio_reader_pcm(
                reader,
                writer,
                prefix(transform, rate),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_aac_export_routing::export_with_router(
        router,
        spool,
        destination,
        transform,
        options,
    )
}

pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    geometry_for_decode(source, None, options)
}

pub(crate) fn geometry_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32)> {
    let (rate, channels, mask, _) = geometry_and_frames_for_decode(source, transform, options)?;
    Ok((rate, channels, mask))
}

pub(crate) fn geometry_and_frames_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64)> {
    let (rate, channels, mask, frames, _) =
        geometry_frames_and_info_for_decode(source, transform, options)?;
    Ok((rate, channels, mask, frames))
}

pub(crate) fn geometry_frames_and_info_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64, Vec<u8>)> {
    let reader = crate::owned_matroska_audio::open_aac_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index = crate::owned_matroska_audio::matroska_audio_index(&reader, selected)
        .map_err(|e| e.to_string())?;
    geometry_from_reader_for_decode(source, reader, index, transform, options)
}

pub(crate) fn geometry_from_reader_for_decode<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64, Vec<u8>)> {
    let (rate, channels, mask, estimated_frames, info, _) =
        geometry_and_reader_for_decode(source, reader, index, transform, options)?;
    Ok((rate, channels, mask, estimated_frames, info))
}

pub(crate) fn geometry_and_reader_for_decode<R: Read + std::io::Seek>(
    source: &Path,
    mut reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64, Vec<u8>, crate::owned_webm::WebmReader<R>)> {
    crate::owned_aac::stream::check_decode_admission(1, options).map_err(|e| e.to_string())?;
    if !reader
        .tracks
        .get(index)
        .is_some_and(|track| track.kind == 2 && track.codec == "A_AAC")
    {
        return Err("selected Matroska audio stream has a different codec".into());
    }
    let track_rate = u32::try_from(reader.tracks[index].sample_rate)
        .map_err(|_| "AAC output clock overflow")?;
    let interval = transform.and_then(|t| prefix(t, track_rate));
    crate::owned_matroska_audio::admit_audio_reader_with_interval(&mut reader, index, interval, options)
        .map_err(|e| e.to_string())?;
    let in_band_ps = crate::owned_matroska_audio::negotiate_matroska_ps_with_interval(
        &mut reader,
        index,
        interval,
        options.max_packets,
        || {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(crate::owned_matroska_audio::invalid("media operation cancelled"));
            }
            crate::owned_budget::check_rss_budget(options)
                .map_err(|e| crate::owned_matroska_audio::invalid(&e))
        },
    )
    .map_err(|e| e.to_string())?;
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let decoder = if in_band_ps {
        crate::owned_matroska_audio::MatroskaTimelineDecoder::with_in_band_ps(track)
    } else { crate::owned_matroska_audio::MatroskaTimelineDecoder::from_matroska(track) }
        .map_err(|e| e.to_string())?;
    let (rate, channels, mask) = (
        decoder.sample_rate(),
        decoder.channels(),
        decoder.aac_channel_mask().map_err(|e| e.to_string())?,
    );
    if track.kind != 2
        || track.codec != "A_AAC"
        || track.sample_rate != u64::from(rate)
        || (!in_band_ps && track.channels != u64::from(channels))
    {
        return Err("Matroska audio geometry disagrees with configuration or codec".into());
    }
    let mut estimated_frames = crate::owned_matroska_audio::matroska_estimated_frames(
        &mut reader,
        index,
        rate,
        interval,
        options.max_packets,
    );
    let info = crate::owned_wave_metadata::matroska_source_info_from_reader(
        &mut reader, index, interval, options,
    )?;
    if transform.is_some_and(|t| t.interval.is_some()) {
        let mut probe = decode_options(options);
        probe.progress = None;
        let stats = crate::owned_matroska_aac::decode_matroska_aac_pcm(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            &mut std::io::sink(),
            interval,
            &probe,
        )
        .map_err(|e| e.to_string())?;
        crate::owned_ogg_opus_export::check_nonempty_interval_prefix(
            transform,
            stats.sample_rate,
            stats.sample_frames,
        )?;
        estimated_frames = stats.sample_frames;
    }
    Ok((rate, channels, mask, estimated_frames, info, reader))
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn matroska_aac_export_admits_combined_index_and_decoder_policy() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/aac-stereo.mka");
        let output = std::env::temp_dir().join(format!(
            "fvid-matroska-aac-budget-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let small = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &small));
        let error = crate::decode_audio_transformed(&source, &output, Default::default(), &small)
            .unwrap_err();
        assert!(
            error.contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(!output.exists());
        let admitted = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &admitted));
        crate::plan_decode_audio(&source, &Default::default(), &admitted).unwrap();
        crate::decode_audio_transformed(&source, &output, Default::default(), &admitted).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        std::fs::remove_file(&output).unwrap();
        crate::decode_audio_transformed(&source, &output, Default::default(), &Default::default())
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        std::fs::remove_file(&output).unwrap();
        let reader = crate::owned_webm::WebmReader::open(
            BufReader::new(File::open(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        let config =
            crate::owned_aac::config::AacConfig::parse(&reader.tracks[0].codec_private).unwrap();
        let decoder_only = CopyOptions {
            max_controlled_bytes: Some(
                crate::owned_aac::stream::decode_admission_bytes(u16::from(config.channels))
                    .unwrap(),
            ),
            ..Default::default()
        };
        crate::owned_aac::stream::check_decode_admission(u16::from(config.channels), &decoder_only)
            .unwrap();
        drop(reader);
        let mut samples = Vec::new();
        let error = crate::owned_matroska_aac::decode_matroska_aac_pcm(
            BufReader::new(File::open(&source).unwrap()),
            &mut samples,
            None,
            &decoder_only,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded")
        );
        assert!(samples.is_empty());
        let mut pcm = Vec::new();
        let error = crate::owned_matroska_aac::decode_matroska_aac_pcm(
            BufReader::new(File::open(source).unwrap()),
            &mut pcm,
            None,
            &small,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded")
        );
        assert!(pcm.is_empty());
    }
}
