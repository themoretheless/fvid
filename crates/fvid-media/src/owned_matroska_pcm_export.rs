//! Matroska PCM file export via precise float64 presentation decoding and WAVE DSP.
//! All supported integer samples fit exactly in f64; float64 never passes through f32.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut signature = [0; 4];
    if file.read_exact(&mut signature).is_err() || signature != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(false);
    }
    let reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let preferred=reader.preferred_track_numbers(&reader.tracks.iter().filter(|t|t.kind==2).map(|t|t.number).collect::<Vec<_>>());
    Ok(reader.tracks.iter().enumerate().any(|(index, t)| {
        ((options.streams.is_empty() && preferred.contains(&t.number)) || options.streams.contains(&index))
            && t.kind == 2
            && matches!(
                t.codec.as_str(),
                "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
            )
    }))
}
fn decode_options(options: &CopyOptions) -> CopyOptions {
    let mut options = options.clone();
    options.metadata_set.clear();
    options.metadata_delete.clear();
    options.stream_metadata_set.clear();
    options.stream_metadata_delete.clear();
    options
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels) = match geometry_for_decode(source, Some(transform), options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let target_ok = transform.channels.is_none_or(|target| {
        target == i32::from(channels) || (channels <= 2 && (1..=8).contains(&target))
    });
    if !target_ok {
        return false;
    }
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_pcm::decode_matroska_pcm_f64(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok()
        || result
            .err()
            .is_some_and(|e| e.to_string().starts_with("controlled memory budget exceeded:"))
}
pub(crate) fn apply(
    source: &Path,
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
    let reader = crate::owned_matroska_audio::open_audio_reader(
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
    apply_from_reader(source, reader, index, destination, transform, options)
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
    let (rate, channels, frames, info) =
        geometry_from_reader_for_decode(source, reader, index, Some(transform), options)?;
    let mask = if channels <= 2 {
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
    } else {
        0
    };
    crate::owned_adts_export::check_spool_wave_export_budget(
        rate,
        channels,
        64,
        frames.max(1024),
        info.len(),
        false,
        transform,
        options,
    )?;
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        64,
        options,
        |writer, options| {
            crate::owned_matroska_pcm::decode_matroska_pcm_f64(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    geometry_for_decode(source, None, options)
}

pub(crate) fn geometry_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16)> {
    let (rate, channels, _) = geometry_and_frames_for_decode(source, transform, options)?;
    Ok((rate, channels))
}

pub(crate) fn geometry_and_frames_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u64)> {
    let (rate, channels, frames, _) =
        geometry_frames_and_info_for_decode(source, transform, options)?;
    Ok((rate, channels, frames))
}

pub(crate) fn geometry_frames_and_info_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u64, Vec<u8>)> {
    let reader = crate::owned_matroska_audio::open_audio_reader(
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
    mut reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u64, Vec<u8>)> {
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let decoder =
        crate::owned_pcm_decoder::PcmDecoder::from_matroska(track).map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    let interval = transform.and_then(|t| prefix(t, rate));
    crate::owned_matroska_audio::admit_audio_reader_with_interval(&mut reader, index, interval, options)
        .map_err(|e| e.to_string())?;
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
    drop(reader);
    if transform.is_some_and(|t| t.interval.is_some()) {
        let mut probe = decode_options(options);
        probe.progress = None;
        let stats = crate::owned_matroska_pcm::decode_matroska_pcm_f64(
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
    Ok((rate, channels, estimated_frames, info))
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn matroska_pcm_workflows_keep_owned_budget() {
        check_budget("../../tests/fixtures/audio/pcm-int.mkv");
    }
    #[test]
    fn matroska_float64_workflows_keep_owned_budget_and_precision() {
        check_budget("../../tests/fixtures/playback-errors/pcm64-resample-window.mka");
    }
    fn check_budget(fixture: &str) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture);
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        let admitted = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &tiny));
        assert!(crate::owned_audio_plan::supports(
            &source,
            &Default::default(),
            &tiny
        ));
        assert!(crate::owned_container_loudness::supports(&source, &tiny));
        assert!(
            crate::plan_decode_audio(&source, &Default::default(), &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        crate::plan_decode_audio(&source, &Default::default(), &admitted).unwrap();
        assert!(
            crate::measure_loudness(&source, &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        let expected = crate::measure_loudness(&source, &Default::default()).unwrap();
        let actual = crate::measure_loudness(&source, &admitted).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        for mode in 0..3 {
            let output = std::env::temp_dir().join(format!(
                "fvid-matroska-pcm-budget-{}-{mode}-{}.wav",
                source.file_stem().unwrap().to_string_lossy(),
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            let apply = |options: &CopyOptions| -> Result<()> {
                match mode {
                    0 => crate::decode_audio(&source, &output, options).map(|_| ()),
                    1 => crate::apply_loudnorm(&source, &output, None, options).map(|_| ()),
                    _ => crate::apply_loudnorm_dual(&source, &output, None, options).map(|_| ()),
                }
            };
            assert!(
                apply(&tiny)
                    .unwrap_err()
                    .contains("controlled memory budget exceeded")
            );
            assert!(!output.exists());
            apply(&admitted).unwrap();
            let bytes = std::fs::read(&output).unwrap();
            std::fs::remove_file(&output).unwrap();
            apply(&CopyOptions::default()).unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), bytes);
            std::fs::remove_file(&output).unwrap();
        }
    }
}
