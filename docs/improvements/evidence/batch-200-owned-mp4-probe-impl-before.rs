// Source-clock average: edits and presentation gaps do not change source cadence.
fn mp4_average_rate(durations: impl Iterator<Item = Option<u32>>, scale: u32) -> [i32; 2] {
    if scale == 0 {
        return [0, 1];
    }
    let mut count = 0u128;
    let mut total = 0u128;
    for duration in durations {
        let Some(duration) = duration.filter(|&n| n != 0) else {
            return [0, 1];
        };
        count += 1;
        total += u128::from(duration);
    }
    if count == 0 {
        return [0, 1];
    }
    let numerator = count * u128::from(scale);
    let (mut a, mut b) = (numerator, total);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    match (i32::try_from(numerator / a), i32::try_from(total / a)) {
        (Ok(n), Ok(d)) => [n, d],
        _ => [0, 1],
    }
}

fn mp4_payload_rate(samples: impl Iterator<Item = Option<(u32, u32)>>, scale: u32) -> Option<i64> {
    if scale == 0 {
        return None;
    }
    let mut bytes = 0u128;
    let mut ticks = 0u128;
    for sample in samples {
        let (size, duration) = sample?;
        if duration == 0 {
            return None;
        }
        bytes = bytes.checked_add(u128::from(size))?;
        ticks = ticks.checked_add(u128::from(duration))?;
    }
    if ticks == 0 {
        return None;
    }
    i64::try_from(bytes.checked_mul(8)?.checked_mul(u128::from(scale))? / ticks).ok()
}

pub(crate) fn mp4(path: &Path) -> Result<MediaInfo> {
    try_mp4(path)?.ok_or_else(|| {
        "native MP4 probe cannot yet describe every sample entry in this file".into()
    })
}

// None denotes valid tracks whose sample entries are not represented yet.
// Parser errors remain errors, rather than requesting a permissive fallback.
pub(crate) fn try_mp4(path: &Path) -> Result<Option<MediaInfo>> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let file_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let reader = match Mp4ProbeReader::open(BufReader::new(file), Default::default()) {
        Ok(reader) => reader,
        Err(error) if mp4_unrepresented_error(&error) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    // The reader keeps unsupported sample entries separately, without their
    // original stream positions. Do not return an incomplete/reindexed inventory.
    if !reader.refused().is_empty() {
        return Ok(None);
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
            b"raw " => "pcm_u8".into(),
            b"sowt" | b"twos" | b"in24" | b"in32" => {
                let little = track.codec == *b"sowt"
                    || (track.codec != *b"twos" && track.configuration.first() == Some(&1));
                format!(
                    "pcm_s{}{}",
                    track.bit_depth,
                    if little { "le" } else { "be" }
                )
            }
            b"fl32" | b"fl64" => format!(
                "pcm_f{}{}",
                if track.codec == *b"fl32" { 32 } else { 64 },
                if track.configuration.first() == Some(&1) {
                    "le"
                } else {
                    "be"
                }
            ),
            b"ima4" => "adpcm_ima_qt".into(),
            b"ms\x00\x11" => "adpcm_ima_wav".into(),
            b"ms\x00\x02" => "adpcm_ms".into(),
            b"MAC3" => "mace3".into(),
            b"MAC6" => "mace6".into(),
            b"tx3g" => "mov_text".into(),
            b"ac-3" => "ac3".into(),
            b"ec-3" => "eac3".into(),
            _ => String::from_utf8_lossy(&track.codec).into_owned(),
        };
        let extradata_bytes = if track.codec == *b"mp4a" {
            aac_probe_config(&track.configuration)
                .map_err(|e| e.to_string())?
                .len()
        } else {
            track.configuration.len()
        };
        let avc_description = if matches!(&track.codec, b"avc1" | b"avc3") {
            avc_probe_config(&track.configuration)
        } else { None };
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
            bit_rate: mp4_payload_rate(
                (0..track.samples.len())
                    .map(|i| track.samples.get(i).map(|s| (s.size, s.duration))),
                track.timescale,
            ),
            average_frame_rate: if track.handler == *b"vide" {
                mp4_average_rate(
                    (0..track.samples.len()).map(|i| track.samples.get(i).map(|s| s.duration)),
                    track.timescale,
                )
            } else {
                [0, 1]
            },
            profile: avc_description.map(|(name, _)| name.into()),
            level: avc_description.map(|(_, level)| i32::from(level)),
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
    Ok(Some(MediaInfo {
        path: path.to_path_buf(),
        format: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        start_us,
        duration_us,
        bit_rate: duration_us
            .filter(|&duration| duration > 0)
            .and_then(|duration| {
                i64::try_from(u128::from(file_bytes) * 8 * 1_000_000 / duration as u128).ok()
            }),
        metadata: tags(reader.tags()),
        chapters,
        streams,
    }))
}

#[cfg(test)]
mod average_rate_tests {
    use super::mp4_average_rate;
    #[test]
    fn average_uses_total_clock_and_exact_reduced_ratio() {
        assert_eq!(
            mp4_average_rate([Some(3003); 3].into_iter(), 90000),
            [30000, 1001]
        );
        assert_eq!(
            mp4_average_rate([Some(2), Some(3), Some(1)].into_iter(), 6),
            [3, 1]
        );
    }
    #[test]
    fn unknown_or_unrepresentable_timing_is_not_guessed() {
        assert_eq!(mp4_average_rate([].into_iter(), 90000), [0, 1]);
        assert_eq!(mp4_average_rate([None].into_iter(), 90000), [0, 1]);
        assert_eq!(mp4_average_rate([Some(0)].into_iter(), 90000), [0, 1]);
        assert_eq!(mp4_average_rate([Some(1)].into_iter(), 0), [0, 1]);
        assert_eq!(mp4_average_rate([Some(1)].into_iter(), u32::MAX), [0, 1]);
    }
}

#[cfg(test)]
mod payload_rate_tests {
    use super::mp4_payload_rate;
    #[test]
    fn uses_payload_bytes_and_total_source_clock() {
        assert_eq!(
            mp4_payload_rate([Some((264, 128)); 4].into_iter(), 48000),
            Some(792000)
        );
        assert_eq!(
            mp4_payload_rate([Some((1, 2)), Some((2, 3)), Some((3, 1))].into_iter(), 6),
            Some(48)
        );
        assert_eq!(mp4_payload_rate([Some((0, 1))].into_iter(), 6), Some(0));
    }
    #[test]
    fn unknown_clock_and_overflow_remain_unknown() {
        assert_eq!(mp4_payload_rate([].into_iter(), 48000), None);
        assert_eq!(mp4_payload_rate([None].into_iter(), 48000), None);
        assert_eq!(mp4_payload_rate([Some((1, 0))].into_iter(), 48000), None);
        assert_eq!(mp4_payload_rate([Some((1, 1))].into_iter(), 0), None);
        assert_eq!(
            mp4_payload_rate([Some((u32::MAX, 1))].into_iter(), u32::MAX),
            None
        );
    }
}
