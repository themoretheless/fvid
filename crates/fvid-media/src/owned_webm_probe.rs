//! Container-only WebM/Matroska inspection, without codec inference or libav.
use crate::owned_webm::{Limits, WebmReader};
use fvid_media_info::{ChapterInfo, MediaInfo, StreamInfo};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};
pub fn probe_webm(path: &Path) -> Result<MediaInfo, String> {
    try_probe_webm(path)?.ok_or_else(|| "Matroska probe requires supported content encoding and block lacing".into())
}
/// None marks content encodings or block lacing that need parser support; malformed
/// container data stays an error and cannot request a fallback.
pub fn try_probe_webm(path: &Path) -> Result<Option<MediaInfo>, String> {
    let mut reader = match WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?), Limits::default(),
    ) {
        Ok(reader) => reader,
        Err(error) if error.is_unsupported() => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    match reader.scan_all() {
        Ok(()) => {},
        Err(error) if error.is_unsupported() => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let checked = |n: u64| {
        i64::try_from(n).map_err(|_| "Matroska metadata timestamp exceeds API range".to_string())
    };
    let duration = reader.duration_ns.map(checked).transpose()?;
    let mut metadata = reader.metadata.clone();
    let encoder = metadata
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("encoder"))
        .map(|(_, value)| value.clone())
        .or_else(|| (!reader.writing_app.is_empty()).then(|| reader.writing_app.clone()));
    if let Some(encoder) = encoder {
        metadata.entry("encoder".into()).or_insert(encoder);
    }
    let t = &reader.tags;
    for (key, value) in [
        ("title", &t.title),
        ("artist", &t.artist),
        ("album", &t.album),
        ("genre", &t.genre),
        ("date", &t.date),
        ("comment", &t.comment),
        ("track", &t.track),
        ("album_artist", &t.album_artist),
        ("disc", &t.disc),
        ("publisher", &t.publisher),
        ("copyright", &t.copyright),
        ("description", &t.description),
        ("rating", &t.rating),
    ] {
        if !value.is_empty() {
            metadata.insert(key.into(), value.clone());
        }
    }
    let mut streams = Vec::new();
    for (index, track) in reader.tracks.iter().enumerate() {
        let start = reader
            .packets
            .iter()
            .filter(|p| p.track == track.number)
            .map(|p| p.pts_ns)
            .min();
        let codec = match track.codec.as_str() {
            "V_VP8" => "vp8",
            "V_VP9" => "vp9",
            "V_AV1" => "av1",
            "V_MPEG4/ISO/AVC" => "h264",
            "V_MPEGH/ISO/HEVC" => "hevc",
            "V_FFV1" => "ffv1",
            "A_AAC" => "aac",
            "A_OPUS" => "opus",
            "A_VORBIS" => "vorbis",
            "A_ALAC" => "alac",
            "A_FLAC" => "flac",
            other => other,
        };
        let narrow = |n| {
            i32::try_from(n).map_err(|_| "Matroska track dimensions exceed API range".to_string())
        };
        let mut tags = reader.track_metadata.get(&0).cloned().unwrap_or_default();
        if let Some(uid) = reader.track_uids.get(&track.number) {
            if let Some(scoped) = reader.track_metadata.get(uid) {
                tags.extend(scoped.clone());
            }
        }
        for (key, value) in [
            ("title", &track.name),
            (
                "language",
                reader
                    .track_languages
                    .get(&track.number)
                    .unwrap_or(&track.language),
            ),
        ] {
            if !value.is_empty() {
                tags.insert(key.into(), value.clone());
            }
        }
        streams.push(StreamInfo {
            index,
            media_type: match track.kind {
                1 => "video",
                2 => "audio",
                17 => "subtitle",
                _ => "unknown",
            }
            .into(),
            codec: match (track.codec.as_str(),track.bit_depth) {
                ("A_PCM/INT/LIT" | "A_PCM/INT/BIG",8) => "pcm_u8".into(),
                ("A_PCM/INT/LIT",bits @ (16|24|32)) => format!("pcm_s{bits}le"),
                ("A_PCM/INT/BIG",bits @ (16|24|32)) => format!("pcm_s{bits}be"),
                ("A_PCM/FLOAT/IEEE",bits @ (32|64)) => format!("pcm_f{bits}le"),
                _ => codec.into(),
            },
            time_base: [1, 1_000_000_000],
            start,
            duration: None,
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: None,
            level: None,
            disposition: reader
                .track_dispositions
                .get(&track.number)
                .copied()
                .unwrap_or(1),
            metadata: tags,
            width: narrow(track.width)?,
            height: narrow(track.height)?,
            pixel_format: -1,
            sample_rate: narrow(track.sample_rate)?,
            channels: narrow(track.channels)?,
            video_delay: 0,
            extradata_bytes: track.codec_private.len(),
        });
    }
    let mut chapters = Vec::new();
    for (index, chapter) in reader.chapters.iter().enumerate() {
        let start = checked(chapter.start_ns)?;
        // The shared API requires an end; absent end remains a zero-length point.
        let end = chapter.end_ns.map(checked).transpose()?.unwrap_or(start);
        let metadata = if chapter.title.is_empty() {
            BTreeMap::new()
        } else {
            BTreeMap::from([("title".into(), chapter.title.clone())])
        };
        chapters.push(ChapterInfo {
            id: index as i64,
            time_base: [1, 1_000_000_000],
            start,
            end,
            metadata,
        });
    }
    Ok(Some(MediaInfo {
        path: path.into(),
        format: "matroska,webm".into(),
        start_us: streams
            .iter()
            .filter_map(|s| s.start)
            .min()
            .map(|n| n / 1000),
        duration_us: duration.map(|n| n / 1000),
        bit_rate: None,
        metadata,
        chapters,
        streams,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_probe_routes_clear_metadata_and_preserves_encoding_gap() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for (name, codec) in [
            ("webm-probe-clear.mkv", "ffv1"),
            ("pcm64-precision.mka", "pcm_f64le"),
            ("pcm32-precision-little.mka", "pcm_s32le"),
            ("pcm32-precision-big.mka", "pcm_s32be"),
        ] {
            let path = root.join(name);
            let expected = probe_webm(&path).unwrap();
            assert_eq!(expected.streams[0].codec, codec);
            for hint in [None, Some("webm"), Some("matroska"), Some("matroska,webm")] {
                let selected = crate::owned_probe::try_webm_as(&path, hint)
                    .unwrap()
                    .unwrap();
                let public = crate::probe_as(&path, hint).unwrap();
                assert_eq!(
                    serde_json::to_value(selected).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(public).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
            }
            assert!(
                crate::owned_probe::try_webm_as(&path, Some("mov"))
                    .unwrap()
                    .is_none()
            );
        }
        let encoded = root.join("webm-probe-content-encoding.mkv");
        assert!(try_probe_webm(&encoded).unwrap().is_none());
        assert!(
            probe_webm(&encoded)
                .unwrap_err()
                .contains("content encoding")
        );
        let laced = root.join("webm-probe-laced.mkv");
        assert!(try_probe_webm(&laced).unwrap().is_none());
        let broken = root.join("webm-probe-segment-beyond-file.mkv");
        let error = probe_webm(&broken).unwrap_err();
        assert_eq!(error, "EBML element exceeds parent");
        for hint in [None, Some("matroska")] {
            assert_eq!(
                crate::owned_probe::try_webm_as(&broken, hint).unwrap_err(),
                error
            );
            assert_eq!(crate::probe_as(&broken, hint).unwrap_err(), error);
        }
    }
    #[test]
    fn synthetic_container_reaches_owned_probe_dispatch() {
        use crate::owned_matroska::{Chapter, FileMetadata, PacketWriter};
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut file = FileMetadata::default();
        file.tags.title = "Synthetic container".into();
        file.chapters.push(Chapter {
            start_ns: 10_000_000,
            end_ns: Some(30_000_000),
            title: "Chapter".into(),
        });
        let mut writer = PacketWriter::new_ffv1_with_file_metadata(
            &mut bytes, 16, 8, None, 0, 40_000_000, &file,
        )
        .unwrap();
        // Container-only control: the probe must not decode this opaque packet.
        writer
            .write_packet(0, 5_000_000, 40_000_000, true, &[1, 2, 3])
            .unwrap();
        writer.finish().unwrap();
        let path =
            std::env::temp_dir().join(format!("fvid-owned-webm-probe-{}.mkv", std::process::id()));
        std::fs::write(&path, bytes.into_inner()).unwrap();
        for info in [
            probe_webm(&path).unwrap(),
            crate::owned_probe::probe(&path).unwrap(),
            crate::owned_probe::probe_as(&path, Some("matroska")).unwrap(),
            crate::probe(&path).unwrap(),
            crate::probe_as(&path, Some("webm")).unwrap(),
        ] {
            assert_eq!(info.metadata["title"], "Synthetic container");
            assert_eq!(info.streams[0].codec, "ffv1");
            assert_eq!((info.streams[0].width, info.streams[0].height), (16, 8));
            assert_eq!(info.streams[0].start, Some(5_000_000));
            assert_eq!(info.chapters[0].start, 10_000_000);
            assert_eq!(info.chapters[0].end, 30_000_000);
        }
        assert!(crate::owned_probe::probe_as(&path, Some("wav")).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn arbitrary_file_tags_and_writing_app_reach_owned_probe_without_track_tag_leakage() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for name in ["ffv1-custom-tags.mkv", "ffv1-track-tags.mkv"] {
            let info = crate::owned_probe::probe(&root.join(name)).unwrap();
            assert_eq!(info.metadata["FVID_TEST_NOTE"], "own container metadata");
            assert_eq!(info.metadata["ENCODER"], "synthetic source");
            assert_eq!(info.metadata["encoder"], "synthetic source");
            assert_eq!(info.metadata["title"], "Synthetic tags");
            assert!(!info.metadata.contains_key("PRIVATE_TRACK_NOTE"));
            let reader = WebmReader::open(
                BufReader::new(File::open(root.join(name)).unwrap()),
                Limits::default(),
            )
            .unwrap();
            assert_eq!(reader.writing_app, "fvid-synthetic");
        }
    }

}

/// Read a Matroska video's declared nominal cadence, separately from measured average FPS.
/// `DefaultDuration` is in nanoseconds, independent of the segment timestamp scale.
/// Missing durations and non-video streams return `None`; stream indices follow probe order.
pub fn declared_video_frame_rate(path: &Path, stream: usize) -> Result<Option<[i32; 2]>, String> {
    let reader = WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        Limits::default(),
    )
    .map_err(|e| e.to_string())?;
    let track = reader
        .tracks
        .get(stream)
        .ok_or("Matroska stream index out of range")?;
    if track.kind != 1 {
        return Ok(None);
    }
    let rate = crate::owned_time::frame_rate_from_duration_ns(track.default_duration_ns);
    Ok((rate != [0, 1]).then_some(rate))
}
