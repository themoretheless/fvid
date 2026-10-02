//! Container-only WebM/Matroska inspection, without codec inference or libav.
use crate::owned_webm::{Limits, WebmReader};
use fvid_media_info::{ChapterInfo, MediaInfo, StreamInfo};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};
pub fn probe_webm(path: &Path) -> Result<MediaInfo, String> {
    let mut reader = WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        Limits::default(),
    )
    .map_err(|e| e.to_string())?;
    reader.scan_all().map_err(|e| e.to_string())?;
    let checked = |n: u64| {
        i64::try_from(n).map_err(|_| "Matroska metadata timestamp exceeds API range".to_string())
    };
    let duration = reader.duration_ns.map(checked).transpose()?;
    let mut metadata = BTreeMap::new();
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
            other => other,
        };
        let narrow = |n| {
            i32::try_from(n).map_err(|_| "Matroska track dimensions exceed API range".to_string())
        };
        let mut tags = BTreeMap::new();
        for (key, value) in [("title", &track.name), ("language", &track.language)] {
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
            codec: codec.into(),
            time_base: [1, 1_000_000_000],
            start,
            duration: None,
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: None,
            level: None,
            disposition: 0,
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
    Ok(MediaInfo {
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
    })
}
#[cfg(test)]
mod tests {
    use super::*;
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
