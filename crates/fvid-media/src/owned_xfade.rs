//! Integer cross-fade of matching decoded frames, without a media backend.
use crate::owned_y4m::{Header, line};
use crate::{owned_frame::GeometryFrame, owned_overlay::validate_frame};
use std::io::BufRead;

fn default_policy(options: &fvid_control::CopyOptions) -> bool {
    options.streams.is_empty()
        && options.max_packets.is_none()
        && options.max_controlled_bytes.is_none()
        && options.max_rss_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
pub(crate) fn try_xfade_video(
    source: &std::path::Path,
    other: &std::path::Path,
    destination: &std::path::Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    options: &fvid_control::CopyOptions,
) -> Result<Option<fvid_media_info::LosslessStats>, String> {
    if transition != "fade" || !default_policy(options) {
        return Ok(None);
    }
    let read_header = |path: &std::path::Path| -> Option<Header> {
        let mut input = std::io::BufReader::new(std::fs::File::open(path).ok()?);
        let mut bytes = Vec::new();
        line(&mut input, &mut bytes)
            .ok()?
            .then(|| Header::parse(&bytes).ok())
            .flatten()
    };
    let Some(header) = read_header(source) else {
        return Ok(None);
    };
    if read_header(other).is_none() {
        return Ok(None);
    }
    let (count, decoded_frames) =
        export_y4m_ffv1_counted(source, other, destination, offset_us, duration_us, options)?;
    Ok(Some(fvid_media_info::LosslessStats {
        backend: "owned Y4M cross-fade FFV1 export",
        video_frames: count,
        decoded_frames,
        seek_used: false,
        video_packets: count,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: format!("{:?}/{}", header.format, header.depth()),
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: false,
        horizontal_flip: false,
    }))
}
pub fn xfade_video(
    source: &std::path::Path,
    other: &std::path::Path,
    destination: &std::path::Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    options: &fvid_control::CopyOptions,
) -> Result<fvid_media_info::LosslessStats, String> {
    try_xfade_video(
        source,
        other,
        destination,
        transition,
        duration_us,
        offset_us,
        options,
    )?
    .ok_or_else(|| "cross-fade request is not supported by the owned backend".into())
}

/// Forward-only secondary video reader. Retains one frame and selects the most
/// recent frame on its own clock; callers decide how to handle a short source.
pub struct SecondaryReader<R> {
    input: R,
    header: Header,
    current: Option<GeometryFrame>,
    next_index: u64,
    last_target: Option<u64>,
    eof: bool,
    failed: bool,
}
impl<R: BufRead> SecondaryReader<R> {
    pub fn new(mut input: R, primary: &Header) -> Result<Self, String> {
        let mut bytes = Vec::new();
        if !line(&mut input, &mut bytes)? {
            return Err("empty cross-fade secondary source".into());
        }
        let header = Header::parse(&bytes)?;
        header.frame_rate()?;
        if header.width != primary.width
            || header.height != primary.height
            || header.format != primary.format
            || header.depth() != primary.depth()
            || header.full_range()? != primary.full_range()?
        {
            return Err("cross-fade inputs require matching geometry, format and range".into());
        }
        Ok(Self {
            input,
            header,
            current: None,
            next_index: 0,
            last_target: None,
            eof: false,
            failed: false,
        })
    }
    pub fn frame_at(&mut self, pts_ns: u64) -> Result<Option<&GeometryFrame>, String> {
        if self.failed {
            return Err("cross-fade secondary reader requires reset after error".into());
        }
        if let Err(error) = self.advance(pts_ns) {
            self.failed = true;
            self.current = None;
            return Err(error);
        }
        Ok(self.current.as_ref())
    }
    fn advance(&mut self, pts_ns: u64) -> Result<(), String> {
        if self.last_target.is_some_and(|last| pts_ns < last) {
            return Err("cross-fade secondary clock cannot rewind".into());
        }
        self.last_target = Some(pts_ns);
        let [num, den] = self.header.frame_rate()?;
        let mut marker = Vec::new();
        while !self.eof {
            // Compare rationals directly, without rounding the secondary PTS.
            if u128::from(self.next_index) * den as u128 * 1_000_000_000
                > u128::from(pts_ns) * num as u128
            {
                break;
            }
            if !line(&mut self.input, &mut marker)? {
                self.eof = true;
                break;
            }
            if marker != b"FRAME\n" && !marker.starts_with(b"FRAME ") {
                return Err("expected cross-fade Y4M FRAME marker".into());
            }
            let size = self.header.frame_len()?;
            if self.current.is_none() {
                let (sx, sy) = self.header.format.subsampling();
                self.current = Some(GeometryFrame {
                    width: self.header.width,
                    height: self.header.height,
                    subsampling: Some([sx, sy]),
                    data: crate::owned_frame::buffer(size)?,
                });
            }
            self.input
                .read_exact(&mut self.current.as_mut().unwrap().data)
                .map_err(|e| format!("cross-fade secondary payload: {e}"))?;
            self.next_index = self
                .next_index
                .checked_add(1)
                .ok_or("cross-fade frame count overflow")?;
        }
        Ok(())
    }
    pub fn exhausted(&self) -> bool {
        self.eof
    }
    /// Apply a transition on the primary presentation clock. A missing first
    /// secondary frame is an error; a nonempty short source repeats its tail.
    pub fn apply(
        &mut self,
        frame: &mut GeometryFrame,
        depth: u8,
        pts_ns: u64,
        timeline: FadeTimeline,
    ) -> Result<(), String> {
        let (secondary_ns, elapsed, duration) = match timeline.phase(pts_ns) {
            Phase::Primary => return Ok(()),
            Phase::Blend {
                secondary_ns,
                elapsed_ns,
                duration_ns,
            } => (secondary_ns, elapsed_ns, duration_ns),
            Phase::Secondary { secondary_ns } => (secondary_ns, 1, 1),
        };
        let next = self
            .frame_at(secondary_ns)?
            .ok_or("cross-fade secondary source has no frames")?;
        fade(frame, next, depth, elapsed, duration)
    }
}

/// Exact nanosecond timeline for a fade into a secondary source starting at zero.
#[derive(Clone, Copy, Debug)]
pub struct FadeTimeline {
    offset_ns: u64,
    duration_ns: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Primary,
    Blend {
        secondary_ns: u64,
        elapsed_ns: u64,
        duration_ns: u64,
    },
    Secondary {
        secondary_ns: u64,
    },
}
/// Compose matching-rate Y4M streams, including the secondary tail. At most
/// three decoded frame buffers are retained. The callback owns publication.
pub fn visit_y4m(
    main: impl BufRead,
    other: impl BufRead,
    header: &Header,
    timeline: FadeTimeline,
    mut visit: impl FnMut(&GeometryFrame, u64, u64) -> Result<(), String>,
) -> Result<u64, String> {
    visit_y4m_counted(main, other, header, timeline, visit).map(|counts| counts.0)
}
fn visit_y4m_counted(
    main: impl BufRead,
    other: impl BufRead,
    header: &Header,
    timeline: FadeTimeline,
    mut visit: impl FnMut(&GeometryFrame, u64, u64) -> Result<(), String>,
) -> Result<(u64, u64), String> {
    let mut primary = SecondaryReader::new(main, header)?;
    let mut secondary = SecondaryReader::new(other, header)?;
    let rate = header.frame_rate()?;
    if primary.header.frame_rate()? != rate || secondary.header.frame_rate()? != rate {
        return Err("cross-fade inputs require matching frame rates".into());
    }
    let mut count = 0u64;
    loop {
        let tick = u128::from(count) * rate[1] as u128 * 1_000_000_000;
        let end = (u128::from(count) + 1) * rate[1] as u128 * 1_000_000_000;
        let pts =
            u64::try_from(tick / rate[0] as u128).map_err(|_| "cross-fade timestamp overflow")?;
        let sample_pts = u64::try_from(tick.div_ceil(rate[0] as u128))
            .map_err(|_| "cross-fade timestamp overflow")?;
        let duration = u64::try_from(end / rate[0] as u128 - tick / rate[0] as u128)
            .map_err(|_| "cross-fade duration overflow")?;
        let phase = timeline.phase(sample_pts);
        let current = primary
            .frame_at(sample_pts)?
            .ok_or("cross-fade primary source has no frames")?;
        let mut output = GeometryFrame {
            width: current.width,
            height: current.height,
            subsampling: current.subsampling,
            data: current.data.clone(),
        };
        if matches!(phase, Phase::Primary) && primary.eof {
            return Err("cross-fade offset is beyond the primary source".into());
        }
        secondary.apply(&mut output, header.depth(), sample_pts, timeline)?;
        if let Phase::Secondary { secondary_ns } = phase {
            if secondary.eof
                && u128::from(secondary_ns) * rate[0] as u128
                    >= u128::from(secondary.next_index) * rate[1] as u128 * 1_000_000_000
            {
                break;
            }
        }
        visit(&output, pts, duration)?;
        count = count
            .checked_add(1)
            .ok_or("cross-fade output frame count overflow")?;
    }
    Ok((
        count,
        primary
            .next_index
            .checked_add(secondary.next_index)
            .ok_or("cross-fade decoded count overflow")?,
    ))
}
/// Encode and mux a fade without libav. Discard output on any error; file
/// publication and cancellation policy belong to the higher-level exporter.
pub fn write_y4m_ffv1<W: std::io::Write + std::io::Seek>(
    main: impl BufRead,
    other: impl BufRead,
    header: &Header,
    timeline: FadeTimeline,
    output: &mut W,
) -> Result<u64, String> {
    write_y4m_ffv1_counted(main, other, header, timeline, output, &Default::default())
        .map(|counts| counts.0)
}
fn write_y4m_ffv1_counted<W: std::io::Write + std::io::Seek>(
    main: impl BufRead,
    other: impl BufRead,
    header: &Header,
    timeline: FadeTimeline,
    output: &mut W,
    options: &fvid_control::CopyOptions,
) -> Result<(u64, u64, fvid_control::ProgressEvent), String> {
    use crate::owned_matroska::{ColourDescription, PacketWriter, VideoMetadata};
    let width = u32::try_from(header.width).map_err(|_| "cross-fade width overflow")?;
    let height = u32::try_from(header.height).map_err(|_| "cross-fade height overflow")?;
    let metadata = VideoMetadata {
        pixel_aspect: header.pixel_aspect()?,
        colour: Some(ColourDescription {
            matrix: 6,
            full_range: header.full_range()?,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut writer =
        PacketWriter::new_ffv1_with_metadata(output, width, height, Some(&metadata), 0, 0)
            .map_err(|e| e.to_string())?;
    let mut event = fvid_control::ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let count = visit_y4m_counted(main, other, header, timeline, |frame, pts, duration| {
        if options
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.is_cancelled())
        {
            return Err("operation cancelled".into());
        }
        let packet = crate::owned_ffv1_encoder::encode(frame, header.depth())?;
        if packet.len() > options.max_packet_bytes {
            return Err("encoded cross-fade packet exceeds byte limit".into());
        }
        writer
            .write_packet(0, pts, duration, true, &packet)
            .map_err(|e| e.to_string())?;
        event.packets += 1;
        event.payload_bytes = event
            .payload_bytes
            .checked_add(packet.len() as u64)
            .ok_or("cross-fade payload count overflow")?;
        if let Some(progress) = &options.progress {
            progress.emit(event);
        }
        Ok(())
    })?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok((count.0, count.1, event))
}
/// Publish a Y4M fade as FFV1/Matroska without replacing an existing file.
/// Failed decoding or muxing removes the private temporary output.
pub fn export_y4m_ffv1(
    main: &std::path::Path,
    other: &std::path::Path,
    destination: &std::path::Path,
    offset_us: i64,
    duration_us: i64,
) -> Result<u64, String> {
    export_y4m_ffv1_counted(
        main,
        other,
        destination,
        offset_us,
        duration_us,
        &Default::default(),
    )
    .map(|counts| counts.0)
}
fn export_y4m_ffv1_counted(
    main: &std::path::Path,
    other: &std::path::Path,
    destination: &std::path::Path,
    offset_us: i64,
    duration_us: i64,
    options: &fvid_control::CopyOptions,
) -> Result<(u64, u64), String> {
    use std::{fs::File, io::BufReader};
    let timeline = FadeTimeline::new(offset_us, duration_us)?;
    let mut probe = BufReader::new(File::open(main).map_err(|e| e.to_string())?);
    let mut bytes = Vec::new();
    if !line(&mut probe, &mut bytes)? {
        return Err("empty cross-fade primary source".into());
    }
    let header = Header::parse(&bytes)?;
    let input = BufReader::new(File::open(main).map_err(|e| e.to_string())?);
    let other = BufReader::new(File::open(other).map_err(|e| e.to_string())?);
    let (stats, _, decoded_frames) = crate::owned_matroska::export_atomic(
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        |file| {
            let (frames, decoded_frames, event) =
                write_y4m_ffv1_counted(input, other, &header, timeline, file, options)
                    .map_err(crate::owned_matroska::Error)?;
            Ok((
                fvid_media_info::DecodeStats {
                    backend: "owned Y4M cross-fade FFV1 export",
                    video_frames: frames,
                    width: header.width as u32,
                    height: header.height as u32,
                    pixel_format: format!("{:?}/{}", header.format, header.depth()),
                    decode_errors: 0,
                },
                event,
                decoded_frames,
            ))
        },
    )
    .map_err(|e| e.to_string())?;
    Ok((stats.video_frames, decoded_frames))
}
impl FadeTimeline {
    pub fn new(offset_us: i64, duration_us: i64) -> Result<Self, String> {
        if offset_us < 0 || duration_us <= 0 {
            return Err("cross-fade requires nonnegative offset and positive duration".into());
        }
        let offset_ns = (offset_us as u64)
            .checked_mul(1000)
            .ok_or("cross-fade offset overflow")?;
        let duration_ns = (duration_us as u64)
            .checked_mul(1000)
            .ok_or("cross-fade duration overflow")?;
        offset_ns
            .checked_add(duration_ns)
            .ok_or("cross-fade endpoint overflow")?;
        Ok(Self {
            offset_ns,
            duration_ns,
        })
    }
    pub fn phase(self, pts_ns: u64) -> Phase {
        let Some(elapsed) = pts_ns.checked_sub(self.offset_ns) else {
            return Phase::Primary;
        };
        if elapsed >= self.duration_ns {
            Phase::Secondary {
                secondary_ns: elapsed,
            }
        } else {
            Phase::Blend {
                secondary_ns: elapsed,
                elapsed_ns: elapsed,
                duration_ns: self.duration_ns,
            }
        }
    }
}

/// Blend toward `next` by `elapsed / duration`, rounding ties upward.
/// Inputs must share colour encoding and range. Both frames are validated
/// before mutation; frame timing and colour conversion belong to the caller.
pub fn fade(
    frame: &mut GeometryFrame,
    next: &GeometryFrame,
    depth: u8,
    elapsed: u64,
    duration: u64,
) -> Result<(), String> {
    if duration == 0 || elapsed > duration {
        return Err("invalid cross-fade interval".into());
    }
    validate_frame(frame, depth)?;
    validate_frame(next, depth)?;
    if frame.width != next.width
        || frame.height != next.height
        || frame.subsampling != next.subsampling
    {
        return Err("cross-fade requires matching frame geometry".into());
    }
    let blend = |a: u16, b: u16| -> u16 {
        ((u128::from(a) * u128::from(duration - elapsed)
            + u128::from(b) * u128::from(elapsed)
            + u128::from(duration / 2))
            / u128::from(duration)) as u16
    };
    if depth == 8 {
        for (a, b) in frame.data.iter_mut().zip(&next.data) {
            *a = blend(u16::from(*a), u16::from(*b)) as u8;
        }
    } else {
        for (a, b) in frame
            .data
            .chunks_exact_mut(2)
            .zip(next.data.chunks_exact(2))
        {
            let value = blend(
                u16::from_le_bytes([a[0], a[1]]),
                u16::from_le_bytes([b[0], b[1]]),
            );
            a.copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_export_preserves_existing_output_and_cleans_failed_decode() {
        let directory = std::env::temp_dir().join(format!(
            "fvid-xfade-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let main = directory.join("main.y4m");
        let other = directory.join("other.y4m");
        let output = directory.join("output.mkv");
        let source = [
            b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\nFRAME\n".as_slice(),
            &[0; 6],
        ]
        .concat();
        std::fs::write(&main, &source).unwrap();
        std::fs::write(&other, &source).unwrap();
        let successful_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = successful_events.clone();
        let published = output.clone();
        let successful_options = fvid_control::CopyOptions {
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                if event.done {
                    assert!(published.is_file());
                }
                observed.lock().unwrap().push(event);
            })),
            ..Default::default()
        };
        let stats = crate::xfade_video(
            &main,
            &other,
            &output,
            "fade",
            500000,
            0,
            &successful_options,
        )
        .unwrap();
        assert_eq!(stats.video_frames, 1);
        assert_eq!(stats.decoded_frames, 2);
        assert_eq!(stats.backend, "owned Y4M cross-fade FFV1 export");
        let observed = successful_events.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert!(!observed[0].done);
        assert!(observed[1].done);
        assert_eq!(observed[0].payload_bytes, observed[1].payload_bytes);
        drop(observed);
        let original = std::fs::read(&output).unwrap();
        assert!(
            export_y4m_ffv1(&main, &other, &output, 0, 500000)
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(std::fs::read(&output).unwrap(), original);
        std::fs::remove_file(&output).unwrap();
        let cancelled = fvid_control::CancelFlag::new();
        let hook_flag = cancelled.clone();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let hook_events = events.clone();
        let options = fvid_control::CopyOptions {
            cancel: Some(cancelled),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                hook_events.lock().unwrap().push(event);
                hook_flag.cancel();
            })),
            ..Default::default()
        };
        assert!(
            crate::xfade_video(&main, &other, &output, "fade", 500000, 0, &options)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(!output.exists());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].packets, 1);
        assert!(events[0].payload_bytes > 0);
        assert!(!events[0].done);
        drop(events);
        let options = fvid_control::CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        };
        assert!(
            crate::xfade_video(&main, &other, &output, "fade", 500000, 0, &options)
                .unwrap_err()
                .contains("packet exceeds byte limit")
        );
        assert!(!output.exists());
        std::fs::write(
            &main,
            [
                b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\nFRAME\n".as_slice(),
                &[0; 6],
            ]
            .concat(),
        )
        .unwrap();
        std::fs::write(
            &other,
            include_bytes!("../../../tests/fixtures/playback-errors/xfade-secondary-truncated.y4m"),
        )
        .unwrap();
        assert!(
            export_y4m_ffv1(&main, &other, &output, 0, 500000)
                .unwrap_err()
                .contains("secondary payload")
        );
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
    }
    #[test]
    fn matroska_export_roundtrips_transition_pixels_and_secondary_tail() {
        let header_bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n";
        let header = Header::parse(header_bytes).unwrap();
        let make = |values: &[u8]| {
            let mut bytes = header_bytes.to_vec();
            for &value in values {
                bytes.extend_from_slice(b"FRAME\n");
                bytes.extend_from_slice(&[value; 6]);
            }
            std::io::Cursor::new(bytes)
        };
        let mut output = std::io::Cursor::new(Vec::new());
        assert_eq!(
            write_y4m_ffv1(
                make(&[0, 0]),
                make(&[100, 200, 240]),
                &header,
                FadeTimeline::new(500000, 500000).unwrap(),
                &mut output
            )
            .unwrap(),
            4
        );
        output.set_position(0);
        let mut reader = crate::owned_webm::WebmReader::open(output, Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 4);
        let mut decoder = crate::owned_ffv1_decoder::Decoder::new(2, 2, 1 << 20).unwrap();
        for (index, expected) in [0, 0, 200, 240].into_iter().enumerate() {
            assert_eq!(reader.packets[index].pts_ns, index as i64 * 500000000);
            assert_eq!(reader.packets[index].duration_ns, Some(500000000));
            let packet = reader.read_packet(index).unwrap();
            assert_eq!(decoder.decode(&packet).unwrap().frame.data, [expected; 6]);
        }
    }
    #[test]
    fn composed_stream_keeps_secondary_tail_and_exact_frame_clock() {
        let header_bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n";
        let header = Header::parse(header_bytes).unwrap();
        let make = |values: &[u8]| {
            let mut bytes = header_bytes.to_vec();
            for &value in values {
                bytes.extend_from_slice(b"FRAME\n");
                bytes.extend_from_slice(&[value; 6]);
            }
            std::io::Cursor::new(bytes)
        };
        let mut result = Vec::new();
        let count = visit_y4m(
            make(&[0, 0]),
            make(&[100, 200, 240]),
            &header,
            FadeTimeline::new(500000, 500000).unwrap(),
            |frame, pts, duration| {
                result.push((frame.data[0], pts, duration));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count, 4);
        assert_eq!(
            result,
            [
                (0, 0, 500000000),
                (0, 500000000, 500000000),
                (200, 1000000000, 500000000),
                (240, 1500000000, 500000000)
            ]
        );
    }
    #[test]
    fn streaming_transition_blends_on_primary_clock_then_uses_secondary_pixels() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let mut bytes = b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n".to_vec();
        for value in [100, 200] {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[value; 6]);
        }
        let mut reader = SecondaryReader::new(std::io::Cursor::new(bytes), &primary).unwrap();
        let timeline = FadeTimeline::new(100000, 400000).unwrap();
        for (pts, expected) in [(0, 0), (100000000, 0), (300000000, 50), (500000000, 200)] {
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: vec![0; 6],
            };
            reader.apply(&mut frame, 8, pts, timeline).unwrap();
            assert_eq!(frame.data, [expected; 6]);
        }
    }
    #[test]
    fn truncated_secondary_frame_poison_is_specific_and_persistent() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let source =
            include_bytes!("../../../tests/fixtures/playback-errors/xfade-secondary-truncated.y4m");
        let mut reader = SecondaryReader::new(std::io::Cursor::new(source), &primary).unwrap();
        assert!(
            reader
                .frame_at(0)
                .unwrap_err()
                .contains("cross-fade secondary payload")
        );
        assert!(
            reader
                .frame_at(0)
                .unwrap_err()
                .contains("requires reset after error")
        );
        assert!(reader.current.is_none());
    }
    #[test]
    fn secondary_reader_selects_exact_fractional_clock_and_retains_last_frame() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let mut bytes = b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n".to_vec();
        for value in [10, 20, 30] {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[value; 6]);
        }
        let mut reader = SecondaryReader::new(std::io::Cursor::new(bytes), &primary).unwrap();
        for (pts, value) in [
            (0, 10),
            (333333333, 10),
            (333333334, 20),
            (666666667, 30),
            (1000000000, 30),
        ] {
            assert_eq!(reader.frame_at(pts).unwrap().unwrap().data, [value; 6]);
        }
        assert!(reader.exhausted());
        assert!(reader.frame_at(0).is_err());
    }
    #[test]
    fn timeline_preserves_submicrosecond_pts_and_secondary_clock() {
        let timeline = FadeTimeline::new(2, 3).unwrap();
        assert_eq!(timeline.phase(1999), Phase::Primary);
        assert_eq!(
            timeline.phase(2000),
            Phase::Blend {
                secondary_ns: 0,
                elapsed_ns: 0,
                duration_ns: 3000
            }
        );
        assert_eq!(
            timeline.phase(3499),
            Phase::Blend {
                secondary_ns: 1499,
                elapsed_ns: 1499,
                duration_ns: 3000
            }
        );
        assert_eq!(
            timeline.phase(5000),
            Phase::Secondary { secondary_ns: 3000 }
        );
        assert_eq!(
            timeline.phase(u64::MAX),
            Phase::Secondary {
                secondary_ns: u64::MAX - 2000
            }
        );
        assert!(FadeTimeline::new(-1, 3).is_err());
        assert!(FadeTimeline::new(0, 0).is_err());
        assert!(FadeTimeline::new(i64::MAX, 1).is_err());
        assert!(FadeTimeline::new((u64::MAX / 1000) as i64, 1).is_err());
    }
    fn rgb(values: [u8; 3]) -> GeometryFrame {
        GeometryFrame {
            width: 1,
            height: 1,
            subsampling: None,
            data: values.to_vec(),
        }
    }
    #[test]
    fn endpoints_midpoint_and_full_timestamp_range() {
        let next = rgb([255, 100, 1]);
        for (elapsed, duration, expected) in [
            (0, 2, [0, 0, 0]),
            (1, 2, [128, 50, 1]),
            (2, 2, [255, 100, 1]),
            (u64::MAX, u64::MAX, [255, 100, 1]),
        ] {
            let mut frame = rgb([0, 0, 0]);
            fade(&mut frame, &next, 8, elapsed, duration).unwrap();
            assert_eq!(frame.data, expected);
        }
    }
    #[test]
    fn all_planar_depths_preserve_samples_and_fail_atomically() {
        for depth in 8..=16 {
            let maximum = ((1u32 << depth) - 1) as u16;
            let bytes = |value: u16| {
                if depth == 8 {
                    vec![value as u8; 3]
                } else {
                    value.to_le_bytes().repeat(3)
                }
            };
            let mut frame = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([2, 2]),
                data: bytes(0),
            };
            let mut next = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([2, 2]),
                data: bytes(maximum),
            };
            fade(&mut frame, &next, depth, 1, 2).unwrap();
            assert_eq!(frame.data, bytes(maximum / 2 + 1));
            let saved = frame.data.clone();
            next.data.pop();
            assert!(fade(&mut frame, &next, depth, 1, 2).is_err());
            assert_eq!(frame.data, saved);
            let copy = frame_copy(&frame);
            assert!(fade(&mut frame, &copy, depth, 0, 0).is_err());
        }
    }
    fn frame_copy(frame: &GeometryFrame) -> GeometryFrame {
        GeometryFrame {
            width: frame.width,
            height: frame.height,
            subsampling: frame.subsampling,
            data: frame.data.clone(),
        }
    }
}
