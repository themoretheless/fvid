//! Owned MP4 AAC/ALAC file export through presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn open(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_mp4::Mp4Reader<BufReader<File>>> {
    crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() || !crate::owned_mp4::recognizes_prefix(&prefix) {
        return Ok(false);
    }
    let reader = open(source, options)?;
    Ok(reader.tracks().iter().enumerate().any(|(index, t)| {
        (options.streams.is_empty() || options.streams.contains(&index))
            && t.handler == *b"soun"
            && matches!(&t.codec, b"mp4a" | b"alac")
    }))
}
pub(crate) fn descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, String)> {
    let reader = open(source, options)?;
    if !reader.refused().is_empty() {
        return Err("MP4 stream selection requires every track to be represented".into());
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index =
        crate::owned_mp4_audio::mp4_audio_index(&reader, selected).map_err(|e| e.to_string())?;
    crate::owned_mp4_audio::admit_aac_reader(&reader, index, options).map_err(|e| e.to_string())?;
    let track = &reader.tracks()[index];
    let decoder =
        crate::owned_mp4_audio::Mp4TimelineDecoder::new(track).map_err(|e| e.to_string())?;
    if track.timescale == 0
        || track.sample_rate != decoder.sample_rate()
        || track.channels != decoder.channels()
    {
        return Err("MP4 audio export requires valid clock and matching audio geometry".into());
    }
    Ok((
        index,
        decoder.sample_rate(),
        decoder.channels(),
        decoder.channel_mask(),
        if track.codec == *b"mp4a" {
            "aac"
        } else {
            "alac"
        }
        .into(),
    ))
}
fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let (_, rate, channels, mask, _) = descriptor(source, options)?;
    Ok((rate, channels, mask))
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, mask) = match geometry(source, options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let output = transform.channels.unwrap_or(i32::from(channels));
    if output != i32::from(channels)
        && (!((channels <= 2 && (1..=8).contains(&output))
            || (channels <= 8 && matches!(output, 1 | 2)))
            || !crate::owned_pcm_channels::standard_mask(channels)
                .is_some_and(|standard| standard == u64::from(mask)))
    {
        return false;
    }
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut probe = options.clone();
    probe.progress = None;
    probe.metadata_set.clear();
    probe.metadata_delete.clear();
    probe.stream_metadata_set.clear();
    probe.stream_metadata_delete.clear();
    crate::owned_mp4_audio::decode_mp4_audio_pcm(
        BufReader::new(file),
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    )
    .is_ok()
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
    let (rate, channels, mask) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            crate::owned_mp4_audio::decode_mp4_audio_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                options,
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn mp4_aac_export_loudness_and_normalization_keep_owned_budget() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/audio/aac-native-edit.m4a");
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
                "fvid-mp4-aac-budget-{mode}-{}.wav",
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
