//! Matroska ALAC file export via owned presentation decoding and WAVE DSP.
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
    let reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(reader.tracks.iter().enumerate().any(|(index, t)| {
        (options.streams.is_empty() || options.streams.contains(&index))
            && t.kind == 2
            && t.codec == "A_ALAC"
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
    let (rate, _) = match geometry(source, options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_alac::decode_matroska_alac_pcm(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok_and(|stats| {
        transform.channels.is_none_or(|channels| {
            channels == i32::from(stats.channels) || (1..=8).contains(&channels)
        })
    })
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let (rate, channels) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
        options,
        |writer, options| {
            crate::owned_matroska_alac::decode_matroska_alac_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    let mut reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        crate::owned_webm::Limits::default().packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let index = match options.streams.as_slice() {
        [index] => *index,
        [] => {
            let indices: Vec<_> = reader
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (t.kind == 2).then_some(i))
                .collect();
            if indices.len() != 1 {
                return Err("select exactly one audio stream".into());
            }
            indices[0]
        }
        _ => return Err("select exactly one audio stream".into()),
    };
    crate::owned_matroska_audio::admit_audio_reader(&mut reader, index, options)
        .map_err(|e| e.to_string())?;
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let decoder =
        crate::owned_alac::AlacDecoder::from_matroska(track).map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    Ok((rate, channels))
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn matroska_alac_workflows_keep_owned_budget() {
        check_budget("../../tests/fixtures/playback-errors/alac-resample-window.mka");
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
                "fvid-matroska-alac-budget-{}-{mode}-{}.wav",
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
