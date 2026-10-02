pub(crate) fn mp4(path: &Path) -> Result<MediaInfo> {
    let reader = Mp4ProbeReader::open(
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
            b"raw " => "pcm_u8".into(),
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
