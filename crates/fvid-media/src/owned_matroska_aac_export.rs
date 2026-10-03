//! Matroska AAC file export via owned presentation decoding and WAVE DSP.
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
            && t.codec == "A_AAC"
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
    let (rate, _, mask) = match geometry(source, options) {
        Ok(geometry) => geometry,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_aac::decode_matroska_aac_pcm(
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
            channels == i32::from(stats.channels)
                || (((stats.channels <= 2 && (1..=8).contains(&channels))
                    || (stats.channels <= 8 && matches!(channels, 1 | 2)))
                    && crate::owned_pcm_channels::standard_mask(stats.channels)
                        .is_some_and(|standard| standard == u64::from(mask)))
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
    let (rate, channels, mask) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            crate::owned_matroska_aac::decode_matroska_aac_pcm(
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

pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let mut reader = crate::owned_matroska_audio::open_aac_reader(
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
    if !reader
        .tracks
        .get(index)
        .is_some_and(|track| track.kind == 2 && track.codec == "A_AAC")
    {
        return Err("selected Matroska audio stream has a different codec".into());
    }
    crate::owned_matroska_audio::admit_aac_reader(&mut reader, index, options)
        .map_err(|e| e.to_string())?;
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let decoder =
        crate::owned_aac::NativeAacDecoder::new(&track.codec_private).map_err(|e| e.to_string())?;
    let (rate, channels, mask) = (
        decoder.sample_rate(),
        u16::from(decoder.channels()),
        decoder.channel_mask(),
    );
    if track.kind != 2
        || track.codec != "A_AAC"
        || track.sample_rate != u64::from(rate)
        || track.channels != u64::from(channels)
    {
        return Err("Matroska audio geometry disagrees with configuration or codec".into());
    }
    Ok((rate, channels, mask))
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
