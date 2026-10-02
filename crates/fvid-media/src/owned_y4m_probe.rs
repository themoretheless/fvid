//! Container-only Y4M inspection, sharing the frontend parser implementation.
use fvid_media_info::{MediaInfo, StreamInfo};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
/// Inspect only with FVid-owned parsers. Unknown containers return an error.
pub fn try_y4m(source: &Path) -> Result<Option<MediaInfo>> {
    let mut input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let mut header = Vec::new();
    crate::owned_y4m::line(&mut input, &mut header).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&header).map_err(|_| "Y4M header is not UTF-8")?;
    for token in text.split_whitespace().skip(1) {
        if token.starts_with('C')
            && !matches!(
                token,
                "C420" | "C420jpeg" | "C420mpeg2" | "C420paldv" | "C422" | "C444"
            )
            && !token
                .strip_prefix('C')
                .and_then(|v| v.split_once('p'))
                .is_some_and(|(layout, depth)| {
                    matches!(layout, "420" | "422" | "444")
                        && matches!(depth, "9" | "10" | "12" | "14" | "16")
                })
        {
            return Ok(None);
        }
        if token.starts_with('I') && !matches!(token, "Ip" | "I?") {
            return Ok(None);
        }
    }
    probe_y4m(source).map(Some)
}

pub fn probe_y4m(source: &Path) -> Result<MediaInfo> {
    use std::io::{Seek, SeekFrom};
    let mut input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let length = input.get_ref().metadata().map_err(|e| e.to_string())?.len();
    let mut line = Vec::new();
    if !crate::owned_y4m::line(&mut input, &mut line).map_err(|e| e.to_string())? {
        return Err("empty Y4M input".into());
    }
    let header = crate::owned_y4m::Header::parse(&line).map_err(|e| e.to_string())?;
    let frame_bytes = header.frame_len().map_err(|e| e.to_string())? as u64;
    let rate = Some(header.frame_rate()?);
    let mut frames = 0i64;
    while crate::owned_y4m::line(&mut input, &mut line).map_err(|e| e.to_string())? {
        if line != b"FRAME\n" && !line.starts_with(b"FRAME ") {
            return Err("expected Y4M FRAME marker".into());
        }
        let end = input
            .stream_position()
            .map_err(|e| e.to_string())?
            .checked_add(frame_bytes)
            .ok_or("Y4M offset overflow")?;
        if end > length {
            return Err("truncated Y4M frame payload".into());
        }
        input
            .seek(SeekFrom::Start(end))
            .map_err(|e| e.to_string())?;
        frames = frames.checked_add(1).ok_or("Y4M frame count overflow")?;
    }
    let duration_us = rate
        .map(|[n, d]| {
            i64::try_from(
                (i128::from(frames) * i128::from(d) * 1_000_000 + i128::from(n) / 2)
                    / i128::from(n),
            )
            .map_err(|_| "Y4M duration overflow".to_string())
        })
        .transpose()?;
    Ok(MediaInfo {
        path: source.to_path_buf(),
        format: "yuv4mpegpipe".into(),
        start_us: Some(0),
        duration_us,
        bit_rate: None,
        metadata: Default::default(),
        chapters: vec![],
        streams: vec![StreamInfo {
            index: 0,
            media_type: "video".into(),
            codec: "rawvideo".into(),
            time_base: rate.map_or([0, 1], |[n, d]| [d, n]),
            start: Some(0),
            duration: rate.map(|_| frames),
            bit_rate: None,
            average_frame_rate: rate.unwrap_or([0, 1]),
            profile: None,
            level: None,
            disposition: 0,
            metadata: Default::default(),
            width: i32::try_from(header.width).map_err(|_| "Y4M width exceeds API range")?,
            height: i32::try_from(header.height).map_err(|_| "Y4M height exceeds API range")?,
            pixel_format: -1,
            sample_rate: 0,
            channels: 0,
            video_delay: 0,
            extradata_bytes: 0,
        }],
    })
}
