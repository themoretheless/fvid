//! Container-only descriptions, without opening an external demuxer/decoder.
use crate::media::{ChapterInfo, MediaInfo, Result, StreamInfo};
use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

fn ticks(value: u128, scale: u32, target: u32) -> Result<i64> {
    if scale == 0 {
        return Err("zero MP4 timescale".into());
    }
    i64::try_from(
        value
            .checked_mul(u128::from(target))
            .ok_or("MP4 duration overflow")?
            / u128::from(scale),
    )
    .map_err(|_| "MP4 duration exceeds probe API range".into())
}
fn tags(tags: &crate::container::FileTags) -> BTreeMap<String, String> {
    [
        ("title", &tags.title),
        ("artist", &tags.artist),
        ("album", &tags.album),
        ("genre", &tags.genre),
        ("date", &tags.date),
        ("comment", &tags.comment),
        ("track", &tags.track),
        ("album_artist", &tags.album_artist),
        ("disc", &tags.disc),
        ("publisher", &tags.publisher),
        ("copyright", &tags.copyright),
        ("description", &tags.description),
        ("rating", &tags.rating),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key.into(), value.clone()))
    .collect()
}

pub(crate) fn mp4(path: &Path) -> Result<MediaInfo> {
    let reader = crate::container::mp4::Mp4Reader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    // The reader keeps unsupported sample entries separately, without their
    // original stream positions. Do not return an incomplete/reindexed inventory.
    if !reader.refused().is_empty() {
        return Err("native MP4 probe cannot yet describe every sample entry in this file".into());
    }
    let mut streams = Vec::new();
    let mut duration_us = None::<i64>;
    let mut start_us = None::<i64>;
    for (index, track) in reader.tracks().iter().enumerate() {
        let scale =
            i32::try_from(track.timescale).map_err(|_| "MP4 clock exceeds probe API range")?;
        if scale <= 0 {
            return Err("zero MP4 timescale".into());
        }
        let edited = !track.edits.is_empty();
        let (duration, presentation_us) = if edited {
            let movie_duration = track.edits.iter().try_fold(0u128, |sum, edit| {
                sum.checked_add(u128::from(edit.duration))
                    .ok_or("MP4 edit duration overflow")
            })?;
            (
                ticks(movie_duration, reader.movie_timescale(), track.timescale)?,
                ticks(movie_duration, reader.movie_timescale(), 1_000_000)?,
            )
        } else {
            (
                i64::try_from(track.duration)
                    .map_err(|_| "MP4 duration exceeds probe API range")?,
                ticks(u128::from(track.duration), track.timescale, 1_000_000)?,
            )
        };
        let start = if edited {
            Some(0)
        } else {
            (0..track.samples.len())
                .filter_map(|i| track.samples.get(i).map(|s| s.pts))
                .min()
        };
        if let Some(start) = start {
            let us = i64::try_from(i128::from(start) * 1_000_000 / i128::from(scale))
                .map_err(|_| "MP4 start exceeds probe API range")?;
            start_us = Some(start_us.map_or(us, |old| old.min(us)));
        }
        duration_us = Some(duration_us.map_or(presentation_us, |old| old.max(presentation_us)));
        let mut metadata = BTreeMap::new();
        if !track.name.is_empty() {
            metadata.insert("title".into(), track.name.clone());
        }
        if !track.language.is_empty() {
            metadata.insert("language".into(), track.language.clone());
        }
        let codec = match &track.codec {
            b"avc1" | b"avc3" => "h264".into(),
            b"hvc1" | b"hev1" => "hevc".into(),
            b"mp4a" => "aac".into(),
            b"vp09" => "vp9".into(),
            b"av01" => "av1".into(),
            b"text" => "bin_data".into(),
            b"tx3g" => "mov_text".into(),
            b"ac-3" => "ac3".into(),
            b"ec-3" => "eac3".into(),
            _ => String::from_utf8_lossy(&track.codec).into_owned(),
        };
        let extradata_bytes = if track.codec == *b"mp4a" {
            crate::codec::config::aac_specific_config(&track.configuration)
                .map_err(|e| e.to_string())?
                .len()
        } else {
            track.configuration.len()
        };
        streams.push(StreamInfo {
            index,
            media_type: if track.codec == *b"text" {
                "data"
            } else {
                match &track.handler {
                    b"vide" => "video",
                    b"soun" => "audio",
                    b"text" | b"sbtl" | b"subt" => "subtitle",
                    _ => "data",
                }
            }
            .into(),
            codec,
            time_base: [1, scale],
            start,
            duration: Some(duration),
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: None,
            level: None,
            disposition: 0,
            metadata,
            width: i32::from(track.width),
            height: i32::from(track.height),
            pixel_format: -1,
            sample_rate: i32::try_from(track.sample_rate)
                .map_err(|_| "MP4 audio rate exceeds API range")?,
            channels: i32::from(track.channels),
            video_delay: 0,
            extradata_bytes,
        });
    }
    let chapters = reader
        .chapters()
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            let start = i64::try_from(chapter.start_ns / 1000)
                .map_err(|_| "MP4 chapter time exceeds API range")?;
            let end = match reader.chapters().get(index + 1) {
                Some(next) => i64::try_from(next.start_ns / 1000)
                    .map_err(|_| "MP4 chapter time exceeds API range")?,
                None => duration_us.unwrap_or(start).max(start),
            };
            Ok(ChapterInfo {
                id: index as i64,
                time_base: [1, 1_000_000],
                start,
                end,
                metadata: [("title".into(), chapter.title.clone())]
                    .into_iter()
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MediaInfo {
        path: path.to_path_buf(),
        format: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        start_us,
        duration_us,
        bit_rate: None,
        metadata: tags(reader.tags()),
        chapters,
        streams,
    })
}

pub(crate) fn matroska(path: &Path) -> Result<MediaInfo> {
    let mut reader = crate::container::webm::WebmReader::open(
        BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    // Tags/chapters can follow clusters. Complete the bounded index before
    // describing the file, without decoding or retaining encoded payloads.
    reader.scan_all().map_err(|e| e.to_string())?;
    let duration_us = reader
        .duration_ns
        .map(|ns| {
            i64::try_from(ns / 1000).map_err(|_| "Matroska duration exceeds API range".to_owned())
        })
        .transpose()?;
    let mut start_ns = None::<i64>;
    let mut streams = Vec::with_capacity(reader.tracks.len());
    for (index, track) in reader.tracks.iter().enumerate() {
        let start = reader
            .packets
            .iter()
            .filter(|p| p.track == track.number)
            .map(|p| p.pts_ns)
            .min()
            .map(|pts| {
                i64::try_from(i128::from(pts) - i128::from(track.codec_delay_ns))
                    .map_err(|_| "Matroska start exceeds API range".to_owned())
            })
            .transpose()?;
        if let Some(start) = start {
            start_ns = Some(start_ns.map_or(start, |old| old.min(start)));
        }
        let mut metadata = BTreeMap::new();
        if !track.name.is_empty() {
            metadata.insert("title".into(), track.name.clone());
        }
        if !track.language.is_empty() {
            metadata.insert("language".into(), track.language.clone());
        }
        let codec = match track.codec.as_str() {
            "V_MPEG4/ISO/AVC" => "h264",
            "V_MPEGH/ISO/HEVC" => "hevc",
            "V_VP8" => "vp8",
            "V_VP9" => "vp9",
            "V_AV1" => "av1",
            "A_AAC" => "aac",
            "A_OPUS" => "opus",
            "A_VORBIS" => "vorbis",
            "A_FLAC" => "flac",
            "A_AC3" => "ac3",
            "A_EAC3" => "eac3",
            "A_MPEG/L3" => "mp3",
            "A_MPEG/L2" => "mp2",
            "S_TEXT/UTF8" => "subrip",
            "S_TEXT/ASS" => "ass",
            "S_TEXT/SSA" => "ssa",
            other => other,
        }
        .to_owned();
        // DefaultDuration is a declared nominal cadence, not a measured average.
        // Leave average_frame_rate unknown instead of substituting that value.
        streams.push(StreamInfo {
            index,
            media_type: match track.kind {
                1 => "video",
                2 => "audio",
                17 => "subtitle",
                _ => "data",
            }
            .into(),
            codec,
            time_base: [1, 1_000_000_000],
            start,
            duration: None,
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: None,
            level: None,
            disposition: 0,
            metadata,
            width: i32::try_from(track.width).map_err(|_| "Matroska width exceeds API range")?,
            height: i32::try_from(track.height).map_err(|_| "Matroska height exceeds API range")?,
            pixel_format: -1,
            sample_rate: i32::try_from(track.sample_rate)
                .map_err(|_| "Matroska sample rate exceeds API range")?,
            channels: i32::try_from(track.channels)
                .map_err(|_| "Matroska channels exceed API range")?,
            video_delay: 0,
            extradata_bytes: track.codec_private.len(),
        });
    }
    let chapters = reader
        .chapters
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            let start = i64::try_from(chapter.start_ns / 1000)
                .map_err(|_| "Matroska chapter exceeds API range")?;
            let end = if let Some(end) = chapter.end_ns {
                i64::try_from(end / 1000).map_err(|_| "Matroska chapter exceeds API range")?
            } else {
                match reader.chapters.get(index + 1) {
                    Some(next) => i64::try_from(next.start_ns / 1000)
                        .map_err(|_| "Matroska chapter exceeds API range")?,
                    None => duration_us.unwrap_or(start).max(start),
                }
            };
            Ok(ChapterInfo {
                id: index as i64,
                time_base: [1, 1_000_000],
                start,
                end,
                metadata: [("title".into(), chapter.title.clone())]
                    .into_iter()
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MediaInfo {
        path: path.to_path_buf(),
        format: "matroska,webm".into(),
        start_us: start_ns.map(|ns| ns / 1000),
        duration_us,
        bit_rate: None,
        metadata: tags(&reader.tags),
        chapters,
        streams,
    })
}
