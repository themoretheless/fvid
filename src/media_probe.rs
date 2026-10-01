//! Container-only descriptions, without opening an external demuxer/decoder.
pub use fvid_media_info::{ChapterInfo, MediaInfo, StreamInfo};
type Result<T> = std::result::Result<T, String>;
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
            b"raw " => "pcm_u8".into(),
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
            "V_FFV1" => "ffv1",
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

/// Inspect packed PCM without reading or decoding sample payloads.
fn wave(source: &Path) -> Result<MediaInfo> {
    let mut input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let info = crate::native_pcm::inspect(&mut input, None).map_err(|e| e.to_string())?;
    let rate = i32::try_from(info.sample_rate).map_err(|_| "WAVE rate exceeds probe API range")?;
    let duration = i64::try_from(info.sample_frames).map_err(|_| "WAVE duration exceeds probe API range")?;
    let duration_us = i64::try_from(u128::from(info.sample_frames) * 1_000_000 / u128::from(info.sample_rate))
        .map_err(|_| "WAVE duration exceeds probe API range")?;
    let bit_rate = i64::from(info.sample_rate) * i64::from(info.channels) * i64::from(info.bits_per_sample);
    Ok(MediaInfo {
        path: source.to_path_buf(), format: "wav".into(), start_us: Some(0),
        duration_us: Some(duration_us), bit_rate: None,
        metadata: Default::default(), chapters: vec![],
        streams: vec![StreamInfo {
            index: 0, media_type: "audio".into(), codec: info.codec(),
            time_base: [1, rate], start: Some(0), duration: Some(duration),
            bit_rate: Some(bit_rate), average_frame_rate: [0, 1], profile: None,
            level: None, disposition: 0, metadata: Default::default(),
            width: 0, height: 0, pixel_format: -1, sample_rate: rate,
            channels: i32::from(info.channels), video_delay: 0, extradata_bytes: 0,
        }],
    })
}

/// Inspect only with FVid-owned parsers. Unknown containers return an error.
fn try_y4m(source: &Path) -> Result<Option<MediaInfo>> {
    let mut input=BufReader::new(File::open(source).map_err(|e|e.to_string())?);
    let mut header=Vec::new();crate::line(&mut input,&mut header).map_err(|e|e.to_string())?;
    let text=std::str::from_utf8(&header).map_err(|_|"Y4M header is not UTF-8")?;
    for token in text.split_whitespace().skip(1) {
        if token.starts_with('C') && !matches!(token,"C420"|"C420jpeg"|"C420mpeg2"|"C420paldv"|"C422"|"C444")
            && !token.strip_prefix('C').and_then(|v|v.split_once('p')).is_some_and(|(layout,depth)|matches!(layout,"420"|"422"|"444") && matches!(depth,"9"|"10"|"12"|"14"|"16")) {return Ok(None);}
        if token.starts_with('I') && !matches!(token,"Ip"|"I?") {return Ok(None);}
    }
    y4m(source).map(Some)
}

fn y4m(source: &Path) -> Result<MediaInfo> {
    use std::io::{Seek,SeekFrom};
    let mut input=BufReader::new(File::open(source).map_err(|e|e.to_string())?);
    let length=input.get_ref().metadata().map_err(|e|e.to_string())?.len();
    let mut line=Vec::new();
    if !crate::line(&mut input,&mut line).map_err(|e|e.to_string())? {return Err("empty Y4M input".into());}
    let header=crate::Header::parse(&line).map_err(|e|e.to_string())?;
    let frame_bytes=header.frame_len().map_err(|e|e.to_string())? as u64;
    let mut rate=None;
    for token in &header.tokens {
        if let Some(value)=token.strip_prefix('F') {
            if rate.is_some() {return Err("duplicate Y4M frame rate".into());}
            let (n,d)=value.split_once(':').ok_or("invalid Y4M frame rate")?;
            let n=n.parse::<i32>().map_err(|_|"invalid Y4M frame rate")?;
            let d=d.parse::<i32>().map_err(|_|"invalid Y4M frame rate")?;
            if n<=0 || d<=0 {return Err("Y4M frame rate must be positive".into());}
            rate=Some([n,d]);
        }
    }
    let [n,d]=rate.unwrap_or([25,1]);
    let mut a=n;let mut b=d;while b!=0 {let r=a%b;a=b;b=r;}
    let rate=Some([n/a,d/a]);
    let mut frames=0i64;
    while crate::line(&mut input,&mut line).map_err(|e|e.to_string())? {
        if line!=b"FRAME\n" && !line.starts_with(b"FRAME ") {return Err("expected Y4M FRAME marker".into());}
        let end=input.stream_position().map_err(|e|e.to_string())?.checked_add(frame_bytes).ok_or("Y4M offset overflow")?;
        if end>length {return Err("truncated Y4M frame payload".into());}
        input.seek(SeekFrom::Start(end)).map_err(|e|e.to_string())?;
        frames=frames.checked_add(1).ok_or("Y4M frame count overflow")?;
    }
    let duration_us=rate.map(|[n,d]|i64::try_from((i128::from(frames)*i128::from(d)*1_000_000+i128::from(n)/2)/i128::from(n)).map_err(|_|"Y4M duration overflow".to_string())).transpose()?;
    Ok(MediaInfo {path:source.to_path_buf(),format:"yuv4mpegpipe".into(),start_us:Some(0),duration_us,bit_rate:None,metadata:Default::default(),chapters:vec![],streams:vec![StreamInfo {
        index:0,media_type:"video".into(),codec:"rawvideo".into(),time_base:rate.map_or([0,1],|[n,d]|[d,n]),start:Some(0),duration:rate.map(|_|frames),bit_rate:None,average_frame_rate:rate.unwrap_or([0,1]),profile:None,level:None,disposition:0,metadata:Default::default(),width:i32::try_from(header.width).map_err(|_|"Y4M width exceeds API range")?,height:i32::try_from(header.height).map_err(|_|"Y4M height exceeds API range")?,pixel_format:-1,sample_rate:0,channels:0,video_delay:0,extradata_bytes:0,
    }]})
}

pub fn probe(source: &Path) -> Result<MediaInfo> {
    probe_as(source, None)
}
pub fn probe_as(source: &Path, format: Option<&str>) -> Result<MediaInfo> {
    try_probe_as(source, format)?
        .ok_or_else(|| "native probe does not yet support this container".into())
}
/// `None` means no owned parser recognizes the requested format/signature.
/// A recognized but malformed source returns an error, never `None`.
pub fn try_probe_as(source: &Path, format: Option<&str>) -> Result<Option<MediaInfo>> {
    use std::io::{Read, Seek, SeekFrom};
    match format {
        Some("mov" | "mp4" | "m4a") => return mp4(source).map(Some),
        Some("matroska" | "webm") => return matroska(source).map(Some),
        Some("wav") => return wave(source).map(Some),
        Some("yuv4mpegpipe" | "y4m") => return try_y4m(source),
        Some("aac") | None => {}
        Some(_) => return Ok(None),
    }
    let mut input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let mut signature = [0; 12];
    // Read up to twelve bytes, retaining a valid seven-byte ADTS header at EOF.
    let mut count = 0;
    while count < signature.len() {
        match input.read(&mut signature[count..]) {
            Ok(0) => break,
            Ok(n) => count += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    if format.is_none() {
        if signature[..count].starts_with(b"YUV4MPEG2") {return try_y4m(source);}
        if count == 12 && &signature[8..12] == b"WAVE"
            && matches!(&signature[..4], b"RIFF" | b"RIFX" | b"RF64") {
            return wave(source).map(Some);
        }
        if crate::container::mp4::recognizes_prefix(&signature[..count]) {
            return mp4(source).map(Some);
        }
        if signature[..count].starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
            return matroska(source).map(Some);
        }
        if crate::container::adts::header(&signature[..count]).is_none() {
            return Ok(None);
        }
    }
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let info = crate::native_media::inspect_adts(input).map_err(|e| e.to_string())?;
    let duration =
        i64::try_from(info.sample_frames).map_err(|_| "AAC duration exceeds API range")?;
    let duration_us =
        i64::try_from(u128::from(info.sample_frames) * 1_000_000 / u128::from(info.sample_rate))
            .map_err(|_| "AAC duration exceeds API range")?;
    Ok(Some(MediaInfo {
        path: source.to_path_buf(),
        format: "aac".into(),
        start_us: Some(0),
        duration_us: Some(duration_us),
        bit_rate: None,
        metadata: Default::default(),
        chapters: vec![],
        streams: vec![StreamInfo {
            index: 0,
            media_type: "audio".into(),
            codec: "aac".into(),
            time_base: [1, info.sample_rate as i32],
            start: Some(0),
            duration: Some(duration),
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: Some("LC".into()),
            level: None,
            disposition: 0,
            metadata: Default::default(),
            width: 0,
            height: 0,
            pixel_format: -1,
            sample_rate: info.sample_rate as i32,
            channels: i32::from(info.channels),
            video_delay: 0,
            extradata_bytes: 2,
        }],
    }))
}
