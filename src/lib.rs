//! Streaming 8-bit planar YUV processing with CPU and optional GPU backends.
#[cfg(feature = "airbug")]
pub mod airbug_runtime;
pub mod backend;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "media")]
pub use fvid_media as media;
pub mod publish;
pub mod resident;
mod view;
pub use view::{FrameView, PlaneView, RowView};
#[cfg(feature = "gpu")]
mod gpu;
pub use backend::{Backend, ExecutionOptions};
use std::io::{self, BufRead, Write};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(String),
    Gpu(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Invalid(s) => write!(f, "{s}"),
            Self::Gpu(s) => write!(f, "GPU: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
const MAX_LINE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Yuv420,
    Yuv422,
    Yuv444,
}
impl PixelFormat {
    fn subsampling(self) -> (usize, usize) {
        match self {
            Self::Yuv420 => (2, 2),
            Self::Yuv422 => (2, 1),
            Self::Yuv444 => (1, 1),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Header {
    pub width: usize,
    pub height: usize,
    pub format: PixelFormat,
    tokens: Vec<String>,
}
impl Header {
    pub fn parse(line: &[u8]) -> Result<Self> {
        if !line.is_ascii() {
            return Err(invalid("Y4M header must be ASCII"));
        }
        let text = std::str::from_utf8(line).map_err(|_| invalid("Y4M header is not UTF-8"))?;
        let mut words = text.split_whitespace();
        if words.next() != Some("YUV4MPEG2") {
            return Err(invalid("expected YUV4MPEG2 input"));
        }
        let tokens: Vec<String> = words.map(str::to_owned).collect();
        let (mut width, mut height, mut format) = (None, None, None);
        for token in &tokens {
            let value = &token[1..];
            match token.as_bytes()[0] {
                b'W' => {
                    if width.is_some() {
                        return Err(invalid("duplicate width"));
                    }
                    width = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid width"))?,
                    );
                }
                b'H' => {
                    if height.is_some() {
                        return Err(invalid("duplicate height"));
                    }
                    height = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid height"))?,
                    );
                }
                b'C' => {
                    if format.is_some() {
                        return Err(invalid("duplicate pixel format"));
                    }
                    format = Some(match value {
                        "420" | "420jpeg" | "420mpeg2" | "420paldv" => PixelFormat::Yuv420,
                        "422" => PixelFormat::Yuv422,
                        "444" => PixelFormat::Yuv444,
                        _ => return Err(invalid("supported pixel formats: 8-bit 420, 422, 444")),
                    });
                }
                b'I' if value != "p" && value != "?" => {
                    return Err(invalid("interlaced video is not supported"));
                }
                _ => {}
            }
        }
        let header = Self {
            width: width.ok_or_else(|| invalid("missing width"))?,
            height: height.ok_or_else(|| invalid("missing height"))?,
            format: format.unwrap_or(PixelFormat::Yuv420),
            tokens,
        };
        header.frame_len()?;
        Ok(header)
    }
    pub fn frame_len(&self) -> Result<usize> {
        let (sx, sy) = self.format.subsampling();
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(sx)
            || !self.height.is_multiple_of(sy)
        {
            return Err(invalid("dimensions must be positive and chroma-aligned"));
        }
        let y = self
            .width
            .checked_mul(self.height)
            .ok_or_else(|| invalid("dimensions overflow"))?;
        y.checked_add(
            (y / sx / sy)
                .checked_mul(2)
                .ok_or_else(|| invalid("dimensions overflow"))?,
        )
        .ok_or_else(|| invalid("dimensions overflow"))
    }
    fn encode(&self, width: usize, height: usize) -> String {
        let mut s = format!("YUV4MPEG2 W{width} H{height}");
        for t in &self.tokens {
            if !t.starts_with('W') && !t.starts_with('H') {
                s.push(' ');
                s.push_str(t);
            }
        }
        s.push('\n');
        s
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Crop {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Transform {
    pub crop: Option<Crop>,
    pub horizontal: bool,
    pub vertical: bool,
}
#[derive(Debug)]
struct Plane {
    input_offset: usize,
    output_offset: usize,
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}
/// A validated plan. Crop coordinates refer to the input, before either reflection.
#[derive(Debug)]
pub struct Plan {
    planes: Vec<Plane>,
    input_len: usize,
    output_len: usize,
    width: usize,
    height: usize,
    horizontal: bool,
    vertical: bool,
}
impl Plan {
    /// Borrow the transformed pixels without allocating or copying their payload.
    /// Consumers must honor row order and horizontal orientation; exporting packed
    /// bytes may still require writes or materialization. This does not change the
    /// conservative input/output budget admitted by `Plan::new`.
    pub fn view<'a>(&'a self, input: &'a [u8]) -> Result<FrameView<'a>> {
        if input.len() != self.input_len {
            return Err(invalid("frame buffer length does not match plan"));
        }
        Ok(FrameView::new(self, input))
    }
    /// `memory_limit` bounds the combined input and output frame payloads.
    pub fn new(header: &Header, transform: Transform, memory_limit: usize) -> Result<Self> {
        let c = transform.crop.unwrap_or(Crop {
            x: 0,
            y: 0,
            width: header.width,
            height: header.height,
        });
        let (sx, sy) = header.format.subsampling();
        if c.width == 0
            || c.height == 0
            || c.x.checked_add(c.width).is_none_or(|n| n > header.width)
            || c.y.checked_add(c.height).is_none_or(|n| n > header.height)
        {
            return Err(invalid("crop is empty or outside the input frame"));
        }
        if !c.x.is_multiple_of(sx)
            || !c.width.is_multiple_of(sx)
            || !c.y.is_multiple_of(sy)
            || !c.height.is_multiple_of(sy)
        {
            return Err(invalid("crop must be chroma-aligned"));
        }
        let input_len = header.frame_len()?;
        let out = Header {
            width: c.width,
            height: c.height,
            ..header.clone()
        };
        let output_len = out.frame_len()?;
        if input_len
            .checked_add(output_len)
            .is_none_or(|n| n > memory_limit)
        {
            return Err(invalid("frame buffers exceed the configured memory limit"));
        }
        let (mut input_offset, mut output_offset) = (0, 0);
        let mut planes = Vec::with_capacity(3);
        for (dx, dy) in [(1, 1), (sx, sy), (sx, sy)] {
            let (width, height, stride) = (c.width / dx, c.height / dy, header.width / dx);
            planes.push(Plane {
                input_offset,
                output_offset,
                stride,
                x: c.x / dx,
                y: c.y / dy,
                width,
                height,
            });
            input_offset += stride * (header.height / dy);
            output_offset += width * height;
        }
        Ok(Self {
            planes,
            input_len,
            output_len,
            width: c.width,
            height: c.height,
            horizontal: transform.horizontal,
            vertical: transform.vertical,
        })
    }
    #[cfg(any(feature = "gpu", feature = "cuda"))]
    pub(crate) fn gpu_params(&self) -> Result<[u32; 32]> {
        let mut result = [0u32; 32];
        for (index, p) in self.planes.iter().enumerate() {
            for (field, value) in [
                p.input_offset,
                p.output_offset,
                p.stride,
                p.x,
                p.y,
                p.width,
                p.height,
                0,
            ]
            .into_iter()
            .enumerate()
            {
                result[index * 8 + field] = u32::try_from(value)
                    .map_err(|_| invalid("GPU plan exceeds 32-bit indexing"))?;
            }
        }
        result[24] = u32::from(self.horizontal);
        result[25] = u32::from(self.vertical);
        result[26] = u32::try_from(self.input_len)
            .map_err(|_| invalid("GPU input exceeds 32-bit indexing"))?;
        result[27] = u32::try_from(self.output_len)
            .map_err(|_| invalid("GPU output exceeds 32-bit indexing"))?;
        Ok(result)
    }
    pub(crate) fn reuses_input(&self) -> bool {
        !self.horizontal || self.planes.iter().all(|p| p.width == p.stride)
    }
    pub fn apply(&self, input: &[u8], output: &mut [u8]) -> Result<()> {
        if input.len() != self.input_len || output.len() != self.output_len {
            return Err(invalid("frame buffer length does not match plan"));
        }
        for p in &self.planes {
            for row in 0..p.height {
                let source_row = p.y
                    + if self.vertical {
                        p.height - 1 - row
                    } else {
                        row
                    };
                let start = p.input_offset + source_row * p.stride + p.x;
                let src = &input[start..start + p.width];
                let start = p.output_offset + row * p.width;
                let dst = &mut output[start..start + p.width];
                if self.horizontal {
                    fvid_cpu::hflip_row_copy(dst, src, p.width, 1);
                } else {
                    dst.copy_from_slice(src);
                }
            }
        }
        Ok(())
    }
}
/// Read a newline-terminated header without an unbounded allocation.
fn line<R: BufRead>(reader: &mut R, bytes: &mut Vec<u8>) -> Result<bool> {
    bytes.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(false)
            } else {
                Err(invalid("truncated header line"))
            };
        }
        let n = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        if bytes.len() + n > MAX_LINE {
            return Err(invalid("header line exceeds 4096 bytes"));
        }
        let complete = available[n - 1] == b'\n';
        bytes.extend_from_slice(&available[..n]);
        reader.consume(n);
        if complete {
            return Ok(true);
        }
    }
}
fn buffer(size: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(size)
        .map_err(|_| invalid("frame allocation failed"))?;
    result.resize(size, 0);
    Ok(result)
}
#[derive(Debug)]
pub struct Stats {
    pub frames: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub backend: Backend,
    pub device_name: String,
    pub controlled_memory_bytes: usize,
}
/// Pull-based execution: one input frame, with an output frame only when needed.
/// CPU crop/vertical reflection borrow rows; full-width horizontal reflection reverses them in place.
/// A stream error may leave partial output; the CLI adds transactional file publication.
pub fn process<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    transform: Transform,
    memory_limit: usize,
) -> Result<Stats> {
    process_with_options(
        reader,
        writer,
        transform,
        memory_limit,
        ExecutionOptions::default(),
    )
}
/// Process with an explicit backend. GPU allocations and staging are budgeted;
/// backend/driver overhead is not a guaranteed RSS limit.
pub fn process_with_options<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    transform: Transform,
    memory_limit: usize,
    options: ExecutionOptions,
) -> Result<Stats> {
    let mut marker = Vec::new();
    if !line(&mut reader, &mut marker)? {
        return Err(invalid("empty input"));
    }
    let header = Header::parse(&marker)?;
    // Validate geometry first; the selected backend admits its actual allocations.
    let plan = Plan::new(&header, transform, usize::MAX)?;
    let (mut processor, backend, device_name, controlled_memory_bytes) =
        backend::Processor::new(&plan, options, memory_limit)?;
    let borrow_rows = backend == Backend::Cpu && plan.reuses_input();
    let mut input = buffer(plan.input_len)?;
    let mut output = buffer(if borrow_rows { 0 } else { plan.output_len })?;
    writer.write_all(header.encode(plan.width, plan.height).as_bytes())?;
    let mut stats = Stats {
        frames: 0,
        input_bytes: 0,
        output_bytes: 0,
        backend,
        device_name,
        controlled_memory_bytes,
    };
    #[cfg(feature = "cuda")]
    let mut marker_queue: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
    while line(&mut reader, &mut marker)? {
        if marker != b"FRAME\n" && !marker.starts_with(b"FRAME ") {
            return Err(invalid("expected FRAME marker"));
        }
        reader.read_exact(&mut input)?;
        if borrow_rows {
            // This allocation is owned by the streaming loop and is overwritten by
            // the next read. Reverse only the selected crop, leaving padding alone.
            if plan.horizontal {
                for p in &plan.planes {
                    for row in p.y..p.y + p.height {
                        let start = p.input_offset + row * p.stride + p.x;
                        fvid_cpu::hflip_row(&mut input[start..start + p.width], p.width, 1);
                    }
                }
            }
            let view = plan.view(&input)?;
            writer.write_all(&marker)?;
            for (index, p) in plan.planes.iter().enumerate() {
                // Preserve large sequential writes for full-width, forward planes.
                if !plan.vertical && p.width == p.stride {
                    let start = p.input_offset + p.y * p.stride;
                    writer.write_all(&input[start..start + p.width * p.height])?;
                } else if plan.vertical && !plan.horizontal && p.width == p.stride {
                    // Bottom-to-top full-width rows without FrameView overhead.
                    let base = p.input_offset + p.y * p.stride;
                    for row in (0..p.height).rev() {
                        let start = base + row * p.stride;
                        writer.write_all(&input[start..start + p.width])?;
                    }
                } else {
                    let plane = view.plane(index).expect("validated three-plane plan");
                    for row in 0..plane.height() {
                        writer.write_all(plane.row(row).expect("bounded row").storage_bytes())?;
                    }
                }
            }
        } else if backend == Backend::Cuda {
            #[cfg(feature = "cuda")]
            {
                // Depth-2 overlap: keep FRAME markers aligned with completed outputs.
                marker_queue.push_back(marker.clone());
                if processor.cuda_submit(&input, &mut output)? {
                    let done = marker_queue.pop_front().expect("queued marker");
                    writer.write_all(&done)?;
                    writer.write_all(&output)?;
                }
            }
            #[cfg(not(feature = "cuda"))]
            {
                processor.apply(&plan, &input, &mut output)?;
                writer.write_all(&marker)?;
                writer.write_all(&output)?;
            }
        } else {
            processor.apply(&plan, &input, &mut output)?;
            writer.write_all(&marker)?;
            writer.write_all(&output)?;
        }
        stats.frames += 1;
        stats.input_bytes += plan.input_len as u64;
        stats.output_bytes += plan.output_len as u64;
    }
    #[cfg(feature = "cuda")]
    if backend == Backend::Cuda {
        while processor.cuda_flush(&mut output)? {
            let done = marker_queue.pop_front().expect("queued marker");
            writer.write_all(&done)?;
            writer.write_all(&output)?;
        }
        debug_assert!(marker_queue.is_empty());
    }
    writer.flush()?;
    Ok(stats)
}

/// Execute multiple transforms on one resident GPU chain, with host transfers
/// only at Y4M input/output boundaries. Each transform sees the preceding output.
#[cfg(any(feature = "gpu", feature = "cuda"))]
pub fn process_gpu_chain<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    transforms: &[Transform],
    memory_limit: usize,
    options: ExecutionOptions,
) -> Result<(Stats, resident::TransferStats)> {
    let mut marker = Vec::new();
    if !line(&mut reader, &mut marker)? {
        return Err(invalid("empty input"));
    }
    let header = Header::parse(&marker)?;
    let mut pipeline = resident::GpuPipeline::new(&header, transforms, options, memory_limit)?;
    let mut input = buffer(pipeline.input_len())?;
    let mut output = buffer(pipeline.output_len())?;
    let out = pipeline.output_header();
    writer.write_all(header.encode(out.width, out.height).as_bytes())?;
    let mut stats = Stats {
        frames: 0,
        input_bytes: 0,
        output_bytes: 0,
        backend: options.backend,
        device_name: pipeline.device_name().into(),
        controlled_memory_bytes: pipeline.controlled_memory_bytes(),
    };
    while line(&mut reader, &mut marker)? {
        if marker != b"FRAME\n" && !marker.starts_with(b"FRAME ") {
            return Err(invalid("expected FRAME marker"));
        }
        reader.read_exact(&mut input)?;
        pipeline.upload(&input)?.process()?.download(&mut output)?;
        writer.write_all(&marker)?;
        writer.write_all(&output)?;
        stats.frames += 1;
        stats.input_bytes += input.len() as u64;
        stats.output_bytes += output.len() as u64;
    }
    writer.flush()?;
    Ok((stats, pipeline.transfers()))
}

#[cfg(all(test, feature = "media"))]
mod play_controls {
    use std::path::{Path, PathBuf};

    #[test]
    fn rate_scales_time_and_audio_phase() {
        let media = fvid_media::clamp_rate_milli;
        assert_eq!(media(1.0), 1_000);
        assert_eq!(media(0.1), 250);
        assert_eq!(media(8.0), 4_000);
        assert_eq!(media(f32::NAN), 1_000);
        assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 1_000), 1_000_000);
        assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 2_000), 2_000_000);
        assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 250), 250_000);
        assert_eq!(fvid_media::advance_rate_phase(0, 1_000), (1, 0));
        assert_eq!(fvid_media::advance_rate_phase(0, 2_000), (2, 0));
        let mut phase = 0;
        let mut consumed = 0u32;
        for _ in 0..4 {
            let (need, next) = fvid_media::advance_rate_phase(phase, 250);
            consumed += need;
            phase = next;
        }
        assert_eq!(consumed, 1);
        assert_eq!(phase, 0);
    }

    #[test]
    fn playlist_step_stays_inside_the_list() {
        assert_eq!(fvid_media::playlist_step(3, 0, -1), None);
        assert_eq!(fvid_media::playlist_step(3, 0, 1), Some(1));
        assert_eq!(fvid_media::playlist_step(3, 2, 1), None);
        assert_eq!(fvid_media::playlist_step(0, 0, 1), None);
    }

    #[test]
    fn snapshot_bmp_is_bottom_up_bgr() {
        let bytes = fvid_media::encode_bmp(1, 1, &[0x00FF_0000]).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(bytes[54], 0);
        assert_eq!(bytes[55], 0);
        assert_eq!(bytes[56], 255);
        let path = fvid_media::snapshot_path(Path::new("clips/demo.mp4"), 2);
        assert_eq!(path, PathBuf::from("clips/demo-fvid-2.bmp"));
    }

    #[test]
    fn subtitle_text_and_track_cycle() {
        assert_eq!(fvid_media::plain_subtitle("Hello"), "Hello");
        assert_eq!(
            fvid_media::plain_subtitle(r"0,0,Default,,0,0,0,,{\i1}Hello{\i0}\Nthere"),
            "Hello\nthere"
        );
        let (start, end) = fvid_media::subtitle_window(1_000_000, 200, 1200);
        assert_eq!((start, end), (1_200_000, 2_200_000));
        let cues = [fvid_media::SubtitleCue {
            start_us: start,
            end_us: end,
            text: "Hello".into(),
        }];
        assert_eq!(fvid_media::active_subtitle(&cues, 1_200_000), Some("Hello"));
        assert_eq!(fvid_media::active_subtitle(&cues, 2_200_000), None);
        assert_eq!(fvid_media::cycle_track(2, 0, 1, false), 1);
        assert_eq!(fvid_media::cycle_track(2, 1, 1, false), 0);
        assert_eq!(fvid_media::cycle_track(2, 0, 1, true), 1);
        assert_eq!(fvid_media::cycle_track(2, 1, 1, true), -1);
        assert_eq!(fvid_media::cycle_track(2, -1, 1, true), 0);
    }

    #[test]
    fn external_subtitles_and_device_name() {
        let cues = fvid_media::parse_srt(
            "1\n00:00:01,200 --> 00:00:02,000\nHello from file\n\n2\n00:00:02,000 --> 00:00:03,000\nNext\n",
        );
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_us, 1_200_000);
        assert_eq!(cues[0].text, "Hello from file");
        assert_eq!(
            fvid_media::parse_subtitle_clock("0:00:01.20"),
            Some(1_200_000)
        );
        let ass = fvid_media::parse_subtitle_text(
            "[Events]\nDialogue: 0,0:00:01.20,0:00:02.00,Default,,0,0,0,,Hello\n",
        )
        .unwrap();
        assert_eq!(ass[0].text, "Hello");
        let names = vec!["Speakers".into(), "HDMI".into()];
        assert_eq!(fvid_media::find_audio_device(&names, "hdmi"), Some("HDMI"));
        assert_eq!(fvid_media::find_audio_device(&names, " missing "), None);
        assert!(fvid_media::is_playback_url("https://example.test/a.mp4"));
        assert!(fvid_media::is_playback_url("rtsp://cam.example/stream"));
        assert!(fvid_media::is_playback_url("udp://239.1.1.1:5000"));
        assert!(!fvid_media::is_playback_url("file-not-a-url"));
        assert!(!fvid_media::is_playback_url("javascript://alert"));
        let a = fvid_media::ab_mark(None, 1_500_000).unwrap();
        assert_eq!(a.a_us, 1_500_000);
        assert!(a.b_us < 0);
        assert_eq!(fvid_media::ab_restart_us(a, 9_000_000), None);
        let both = fvid_media::ab_mark(Some(a), 4_000_000).unwrap();
        assert_eq!(both.a_us, 1_500_000);
        assert_eq!(both.b_us, 4_000_000);
        assert_eq!(fvid_media::ab_restart_us(both, 3_999_999), None);
        assert_eq!(fvid_media::ab_restart_us(both, 4_000_000), Some(1_500_000));
        let swapped = fvid_media::ab_mark(Some(a), 200_000).unwrap();
        assert_eq!((swapped.a_us, swapped.b_us), (200_000, 1_500_000));
        assert_eq!(fvid_media::ab_mark(Some(both), 0), None);
        let chapters = [0, 10_000_000, 20_000_000];
        assert_eq!(fvid_media::chapter_step(&chapters, 12_000_000, 1), Some(20_000_000));
        assert_eq!(fvid_media::chapter_step(&chapters, 15_000_000, -1), Some(10_000_000));
        assert_eq!(fvid_media::chapter_step(&chapters, 11_000_000, -1), Some(0));
        assert_eq!(fvid_media::chapter_step(&chapters, 25_000_000, 1), None);
        assert_eq!(
            fvid_media::cycle_repeat(fvid_media::RepeatMode::Off),
            fvid_media::RepeatMode::All
        );
        assert_eq!(
            fvid_media::playback_continue(3, 2, fvid_media::RepeatMode::Off),
            fvid_media::PlaybackContinue::Stop
        );
        assert_eq!(
            fvid_media::playback_continue(3, 2, fvid_media::RepeatMode::All),
            fvid_media::PlaybackContinue::Next(0)
        );
        assert_eq!(
            fvid_media::playback_continue(3, 1, fvid_media::RepeatMode::One),
            fvid_media::PlaybackContinue::Restart
        );
        assert_eq!(fvid_media::subtitle_delay_us(0, 2), 100_000);
        assert_eq!(fvid_media::subtitle_clock_us(1_000_000, 100_000), 900_000);
        assert_eq!(fvid_media::audio_delay_frames(50_000, 48_000), 2_400);
        assert_eq!(fvid_media::audio_delay_frames(-50_000, 48_000), -2_400);
        assert_eq!(fvid_media::audio_delay_frames(50_000, 0), 0);
        assert_eq!(fvid_media::step_audio_skew(3, 1, 10), (2, true, 0));
        assert_eq!(fvid_media::step_audio_skew(-3, 1, 10), (-2, false, 1));
        assert_eq!(fvid_media::step_audio_skew(-3, 1, 0), (-3, false, 0));
        assert_eq!(
            fvid_media::cycle_aspect(fvid_media::AspectMode::Source),
            fvid_media::AspectMode::Square
        );
        assert_eq!(fvid_media::cycle_aspect(fvid_media::AspectMode::FiveFour), fvid_media::AspectMode::Source);
        assert_eq!(
            fvid_media::frame_aspect(320, 240, fvid_media::AspectMode::Source),
            (320, 240)
        );
        assert_eq!(
            fvid_media::frame_aspect(320, 240, fvid_media::AspectMode::SixteenNine),
            (16, 9)
        );
        assert_eq!(fvid_media::fit_aspect(1000, 1000, 16, 9), (1000, 562));
        assert_eq!(fvid_media::fit_aspect(1920, 1080, 16, 9), (1920, 1080));
        assert_eq!(fvid_media::clamp_volume_milli(2_500), 2_000);
        assert_eq!(fvid_media::clamp_volume_milli(-5), 0);
        assert_eq!(fvid_media::clamp_volume_milli(1_500), 1_500);
        assert_eq!(fvid_media::center_crop(1920, 1080, 4, 3), (240, 0, 1440, 1080));
        assert_eq!(fvid_media::center_crop(320, 240, 16, 9), (0, 30, 320, 180));
        assert_eq!(
            fvid_media::display_ratio(320, 240, fvid_media::AspectMode::Source, fvid_media::AspectMode::SixteenNine),
            (16, 9)
        );
        assert_eq!(
            fvid_media::display_ratio(320, 240, fvid_media::AspectMode::Square, fvid_media::AspectMode::SixteenNine),
            (1, 1)
        );
        assert_eq!(fvid_media::zoom_step(1_000, true), 2_000);
        assert_eq!(fvid_media::zoom_step(1_000, false), 500);
        assert_eq!(fvid_media::zoom_step(250, false), 250);
        assert_eq!(fvid_media::zoom_step(2_000, true), 2_000);
        assert_eq!(fvid_media::zoom_size(1920, 1080, 1_000), (1920, 1080));
        assert_eq!(fvid_media::zoom_size(1920, 1080, 2_000), (3840, 2160));
        assert_eq!(fvid_media::zoom_label(500), "1:2");
        let mut marks = Vec::new();
        assert!(fvid_media::insert_bookmark(&mut marks, 2_000_000));
        assert!(fvid_media::insert_bookmark(&mut marks, 500_000));
        assert!(!fvid_media::insert_bookmark(&mut marks, 2_000_000));
        assert_eq!(marks[0].media_us, 500_000);
        assert_eq!(fvid_media::bookmark_step(&marks, 0, 1), Some(500_000));
        assert_eq!(fvid_media::bookmark_step(&marks, 500_000, 1), Some(2_000_000));
        assert_eq!(fvid_media::bookmark_step(&marks, 2_000_000, 1), None);
        assert_eq!(fvid_media::bookmark_step(&marks, 2_000_000, -1), Some(500_000));
        let order = fvid_media::shuffled_indices(5, 42);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4]);
        assert_eq!(fvid_media::shuffled_indices(5, 42), order);
        assert_eq!(fvid_media::order_step(&order, 0, 1, false), Some((order[1], 1)));
        assert_eq!(fvid_media::order_step(&[2, 0, 1], 2, 1, false), None);
        assert_eq!(fvid_media::order_step(&[2, 0, 1], 2, 1, true), Some((2, 0)));
        let base = std::path::Path::new(r"D:\lists");
        let items = fvid_media::parse_playlist_text(
            "#EXTM3U\n#EXTINF:1,A\na.mp4\nhttp://example.test/b.mp4\n",
            base,
        );
        assert_eq!(items.len(), 2);
        assert!(items[0].ends_with("a.mp4"));
        assert!(fvid_media::is_playback_url(items[1].to_str().unwrap()));
        let pls = fvid_media::parse_playlist_text(
            "[playlist]\nFile1=c.mp3\nTitle1=C\nNumberOfEntries=1\n",
            base,
        );
        assert!(pls[0].ends_with("c.mp3"));
        assert!(fvid_media::is_hls_playlist("#EXTM3U\n#EXT-X-TARGETDURATION:1\nseg.ts\n"));
        assert!(fvid_media::parse_playlist_text("#EXTM3U\n#EXT-X-TARGETDURATION:1\nseg.ts\n", base).is_empty());
        let cues = [fvid_media::SubtitleCue {
            start_us: 0,
            end_us: 500_000,
            text: "Hi".into(),
        }];
        assert_eq!(
            fvid_media::active_subtitle(&cues, fvid_media::subtitle_clock_us(600_000, 200_000)),
            Some("Hi")
        );
        let corpus = std::fs::read_to_string("docs/PLAYER_CORPUS.md").expect("player corpus");
        let names: Vec<_> = corpus
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("| ")?;
                let (index, rest) = rest.split_once(" | ")?;
                let name = rest.strip_suffix(" |")?;
                index.parse::<usize>().ok()?;
                Some(name.trim())
            })
            .filter(|name| !name.is_empty() && *name != "Name" && *name != "---")
            .collect();
        assert!(
            names.len() >= 500,
            "player corpus has {} entries; need at least 500 real products",
            names.len()
        );
        assert!(names.iter().any(|name| name.eq_ignore_ascii_case("VLC")));
        assert!(names.iter().any(|name| name.eq_ignore_ascii_case("mpv")));
        assert!(names.iter().any(|name| name.eq_ignore_ascii_case("PotPlayer")));
        assert_eq!(fvid_media::clamp_adjust_milli(3_000), 2_000);
        assert_eq!(
            fvid_media::adjust_pixel(40, 80, 120, 1_000, 1_000, 1_000, 1_000),
            (40, 80, 120)
        );
        let bright = fvid_media::adjust_pixel(40, 80, 120, 1_500, 1_000, 1_000, 1_000);
        assert!(bright.0 > 40 && bright.1 > 80 && bright.2 > 120);
        let gray = fvid_media::adjust_pixel(200, 40, 40, 1_000, 1_000, 0, 1_000);
        assert!((gray.0 as i32 - gray.1 as i32).abs() < 8);
        assert!((gray.1 as i32 - gray.2 as i32).abs() < 8);
        let hue = fvid_media::adjust_pixel(200, 40, 40, 1_000, 1_000, 1_000, 1_500);
        assert_ne!(hue, (200, 40, 40));
        assert_eq!(fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), true, false), (0.9, 0.2, 0.1, 0.8));
        assert_eq!(fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), false, true), (0.1, 0.8, 0.9, 0.2));
        assert_eq!(fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), true, true), (0.9, 0.8, 0.1, 0.2));
        assert_eq!(
            fvid_media::cycle_rotate(fvid_media::RotateMode::Deg0),
            fvid_media::RotateMode::Deg90
        );
        assert_eq!(fvid_media::rotate_size(320, 240, fvid_media::RotateMode::Deg90), (240, 320));
        assert_eq!(
            fvid_media::rotate_pixel(0, 0, 320, 240, fvid_media::RotateMode::Deg90),
            (0, 319)
        );
        assert_eq!(
            fvid_media::rotate_pixel(319, 0, 320, 240, fvid_media::RotateMode::Deg90),
            (0, 0)
        );
        assert_eq!(
            fvid_media::rotate_pixel(0, 0, 320, 240, fvid_media::RotateMode::Deg180),
            (319, 239)
        );
        let mut tone = fvid_media::ToneState::default();
        let mut flat = 0.0;
        for _ in 0..64 {
            flat = fvid_media::tone_step(0.25, &mut tone, 1_000, 1_000, 1_000);
        }
        assert!((flat - 0.25).abs() < 0.05);
        let mut boosted = fvid_media::ToneState::default();
        let mut loud = 0.0;
        for _ in 0..64 {
            loud = fvid_media::tone_step(0.25, &mut boosted, 2_000, 1_000, 1_000);
        }
        assert!(loud > flat);
        assert_eq!(fvid_media::EQ_BAND_COUNT, 10);
        assert_eq!(fvid_media::EQ_BAND_HZ[0], 60);
        assert_eq!(fvid_media::EQ_BAND_HZ[9], 16_000);
        let mut geq = fvid_media::GraphicEqState::default();
        let unity = fvid_media::eq_unity_gains();
        let mut flat_eq = 0.0;
        for _ in 0..128 {
            flat_eq = fvid_media::graphic_eq_step(0.25, &mut geq, &unity);
        }
        assert!((flat_eq - 0.25).abs() < 0.05);
        let mut bass_boost = fvid_media::eq_unity_gains();
        bass_boost[0] = 2_000;
        let mut geq_boost = fvid_media::GraphicEqState::default();
        let mut loud_eq = 0.0;
        for _ in 0..128 {
            loud_eq = fvid_media::graphic_eq_step(0.25, &mut geq_boost, &bass_boost);
        }
        assert!(loud_eq > flat_eq);
        assert_eq!(
            fvid_media::average_rgb_pixel(0x00_ff_00_00, 0x00_00_00_00),
            0x00_7f_00_00
        );
        let mut field = vec![0x00_ff_00_00u32, 0x00_00_00_ff, 0x00_00_ff_00, 0x00_ff_ff_00];
        fvid_media::deinterlace_blend_rgb(&mut field, 2, 2);
        assert_eq!(field[0], field[2]);
        assert_eq!(field[1], field[3]);
        assert_eq!(field[0], fvid_media::average_rgb_pixel(0x00_ff_00_00, 0x00_00_ff_00));
        let devices = vec!["Speakers".into(), "HDMI".into(), "USB DAC".into()];
        assert_eq!(
            fvid_media::cycle_output_device(&devices, "HDMI", 1),
            Some("USB DAC")
        );
        assert_eq!(
            fvid_media::cycle_output_device(&devices, "USB DAC", 1),
            Some("Speakers")
        );
        assert_eq!(
            fvid_media::cycle_output_device(&devices, "missing", -1),
            Some("USB DAC")
        );
        assert_eq!(fvid_media::cycle_output_device(&[], "x", 1), None);
        let stats = fvid_media::PlayStats {
            presented_frames: 12,
            skipped_frames: 3,
            width: 640,
            height: 360,
            source_width: 1920,
            source_height: 1080,
            audio: true,
            sample_rate: 48_000,
            channels: 2,
        };
        let line = fvid_media::format_play_stats(&stats, 65_000_000, 120_000_000);
        assert!(line.contains("shown 12"));
        assert!(line.contains("drop 3"));
        assert!(line.contains("1920x1080→640x360"));
        assert!(line.contains("48000 Hz 2ch"));
        let mut frame = [0.25f32, -0.5];
        fvid_media::apply_audio_channel(&mut frame, fvid_media::AudioChannelMode::Mono);
        assert!((frame[0] - (-0.125)).abs() < 1e-6);
        assert_eq!(frame[0], frame[1]);
        let mut swapped = [0.25f32, -0.5];
        fvid_media::apply_audio_channel(&mut swapped, fvid_media::AudioChannelMode::Reverse);
        assert_eq!(swapped, [-0.5, 0.25]);
        assert_eq!(
            fvid_media::cycle_audio_channel(fvid_media::AudioChannelMode::Reverse),
            fvid_media::AudioChannelMode::Stereo
        );
        assert_eq!(fvid_media::clamp_subtitle_margin(500), 400);
        assert_eq!(fvid_media::subtitle_margin_px(12, 40), 52);
        assert_eq!(fvid_media::subtitle_margin_px(12, -20), 0);
        let png = fvid_media::encode_png(1, 1, &[0x00_ff_00_00]).unwrap();
        assert_eq!(&png[0..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(
            fvid_media::cycle_snapshot_format(fvid_media::SnapshotFormat::Bmp),
            fvid_media::SnapshotFormat::Png
        );
        assert_eq!(
            fvid_media::snapshot_path_with_ext(std::path::Path::new("a.mp4"), 1, "png")
                .file_name()
                .and_then(|n| n.to_str()),
            Some("a-fvid-1.png")
        );
        let mut palette = vec![0u8; 1024];
        palette[4..8].copy_from_slice(&[255, 0, 0, 255]);
        assert_eq!(fvid_media::palette_rgba_pixel(1, &palette), 0xff_ff_00_00);
        let pixels = fvid_media::pal8_to_rgba(2, 1, &[1, 0], 2, &palette).unwrap();
        assert_eq!(pixels[0], 0xff_ff_00_00);
        assert_eq!(pixels[1], 0);
        assert_eq!(
            fvid_media::blend_rgba_over_rgb(0x00_00_ff_00, 0x80_ff_00_00),
            0x00_80_7f_00
        );
        let mut frame = vec![0u32; 4];
        let plane = fvid_media::BitmapSubtitle {
            start_us: 0,
            end_us: 1_000_000,
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            pixels: vec![0xff_00_00_ff],
        };
        fvid_media::blit_bitmap_subtitle(&mut frame, 2, 2, &plane);
        assert_eq!(frame[1], 0x00_00_00_ff);
        assert!(fvid_media::active_bitmap_subtitle(std::slice::from_ref(&plane), 10).is_some());
        assert!(fvid_media::active_bitmap_subtitle(std::slice::from_ref(&plane), 2_000_000).is_none());
        assert_eq!(fvid_media::parse_play_clock("90"), Some(90_000_000));
        assert_eq!(fvid_media::parse_play_clock("1:30.0"), Some(90_000_000));
        assert_eq!(fvid_media::parse_play_clock("bad"), None);
        assert_eq!(fvid_media::EQ_PRESET_COUNT, 18);
        assert_eq!(
            fvid_media::eq_preset_gains(fvid_media::EqPreset::Flat),
            fvid_media::eq_unity_gains()
        );
        let rock = fvid_media::eq_preset_db(fvid_media::EqPreset::Rock);
        assert!((rock[0] - 8.0).abs() < 0.01);
        assert!(fvid_media::eq_db_to_milli(0.0) == 1_000);
        assert!(fvid_media::eq_db_to_milli(-6.0) < 1_000);
        assert!(fvid_media::eq_db_to_milli(6.0) > 1_000);
        assert_eq!(
            fvid_media::cycle_eq_preset(fvid_media::EqPreset::Techno),
            fvid_media::EqPreset::Flat
        );
        assert_eq!(fvid_media::seek_step_us(false), fvid_media::SEEK_COARSE_US);
        assert_eq!(fvid_media::seek_step_us(true), fvid_media::SEEK_FINE_US);
        assert_eq!(
            fvid_media::media_display_title(std::path::Path::new("clips/demo.mp4"), Some(" My Title ")),
            "My Title"
        );
        assert_eq!(
            fvid_media::media_display_title(std::path::Path::new("clips/demo.mp4"), None),
            "demo"
        );
        assert_eq!(
            fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Off),
            fvid_media::DeinterlaceMode::Blend
        );
        assert_eq!(
            fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Blend),
            fvid_media::DeinterlaceMode::Bob
        );
        assert_eq!(
            fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Bob),
            fvid_media::DeinterlaceMode::Off
        );
        assert_eq!(fvid_media::deinterlace_label(fvid_media::DeinterlaceMode::Bob), "Bob");
        let mut bob = vec![0x00_ff_00_00u32, 0x00_00_00_ff, 0x00_00_ff_00, 0x00_ff_ff_00];
        fvid_media::apply_deinterlace_rgb(&mut bob, 2, 2, fvid_media::DeinterlaceMode::Bob);
        assert_eq!(bob[0], bob[2]);
        assert_eq!(bob[1], bob[3]);
        let mut unchanged = vec![1u32, 2, 3, 4];
        fvid_media::apply_deinterlace_rgb(&mut unchanged, 2, 2, fvid_media::DeinterlaceMode::Off);
        assert_eq!(unchanged, vec![1, 2, 3, 4]);
        assert_eq!(fvid_media::format_volume_osd(1_000, false), "Volume 100%");
        assert_eq!(fvid_media::format_volume_osd(1_500, false), "Volume 150%");
        assert_eq!(fvid_media::format_volume_osd(500, true), "Volume muted");
        assert_eq!(fvid_media::position_us_from_digit(5, 100_000_000), Some(50_000_000));
        assert_eq!(fvid_media::position_us_from_digit(0, 100_000_000), Some(100_000_000));
        assert_eq!(fvid_media::position_us_from_digit(3, -1), None);
        assert_eq!(fvid_media::media_us_from_fraction(0.25, 80_000_000), 20_000_000);
        assert!((fvid_media::media_fraction(25_000_000, 100_000_000) - 0.25).abs() < f32::EPSILON);
        assert_eq!(fvid_media::volume_step_milli(1_000, fvid_media::VOLUME_STEP_MILLI), 1_050);
        assert_eq!(fvid_media::volume_step_milli(1_990, 50), fvid_media::VOLUME_MAX_MILLI);
        assert_eq!(fvid_media::rate_step_milli(1_000, fvid_media::RATE_STEP_MILLI), 1_100);
        assert_eq!(fvid_media::rate_step_milli(250, -100), 250);
        assert_eq!(fvid_media::format_play_clock(90_000_000), "01:30");
        assert_eq!(fvid_media::format_rate_osd(1_500), "1.50x");
        assert_eq!(
            fvid_media::frame_step_target_us(1_000_000, 40_000, fvid_media::FrameStep::Forward),
            1_040_000
        );
        assert_eq!(
            fvid_media::frame_step_target_us(30_000, 40_000, fvid_media::FrameStep::Backward),
            0
        );
        assert_eq!(
            fvid_media::clamp_balance_milli(3_000),
            fvid_media::BALANCE_MAX_MILLI
        );
        assert_eq!(
            fvid_media::balance_step_milli(fvid_media::BALANCE_CENTER_MILLI, -fvid_media::BALANCE_STEP_MILLI),
            900
        );
        let mut stereo = [0.5f32, 0.5];
        fvid_media::apply_audio_balance(&mut stereo, fvid_media::BALANCE_CENTER_MILLI);
        assert!((stereo[0] - 0.5).abs() < f32::EPSILON && (stereo[1] - 0.5).abs() < f32::EPSILON);
        let mut left = [0.5f32, 0.5];
        fvid_media::apply_audio_balance(&mut left, fvid_media::BALANCE_MIN_MILLI);
        assert!((left[0] - 0.5).abs() < f32::EPSILON && left[1].abs() < f32::EPSILON);
        let mut mono = [0.5f32];
        fvid_media::apply_audio_balance(&mut mono, 0);
        assert!((mono[0] - 0.5).abs() < f32::EPSILON);
        assert_eq!(
            fvid_media::format_balance_osd(fvid_media::BALANCE_CENTER_MILLI),
            "Balance center"
        );
        assert_eq!(fvid_media::clamp_pan_px(0, 100, 100), 0);
        assert_eq!(fvid_media::clamp_pan_px(80, 100, 200), 50);
        assert_eq!(fvid_media::pan_step_px(0, 10, 100, 200), 10);
        let (x, y, w, h) = fvid_media::zoom_pan_rect((0.0, 0.0, 100.0, 100.0), 200.0, 200.0, 0, 0);
        assert!((w - 200.0).abs() < f32::EPSILON && (h - 200.0).abs() < f32::EPSILON);
        assert!((x - (-50.0)).abs() < f32::EPSILON && (y - (-50.0)).abs() < f32::EPSILON);
        assert_eq!(
            fvid_media::clamp_subtitle_scale_milli(3_000),
            fvid_media::SUBTITLE_SCALE_MAX_MILLI
        );
        assert!((fvid_media::subtitle_font_px(22.0, 2_000) - 44.0).abs() < f32::EPSILON);
        assert!(!fvid_media::cycle_eq_bypass(true));
        assert_eq!(fvid_media::format_eq_bypass_osd(false), "EQ on");
        assert_eq!(fvid_media::reset_av_delays(), (0, 0));
        assert_eq!(
            fvid_media::format_delay_osd("audio", 100_000),
            "audio delay 100 ms"
        );
        assert_eq!(fvid_media::volume_from_wheel(1_000, 1), 1_025);
        assert_eq!(fvid_media::volume_from_wheel(0, -1), 0);
        assert_eq!(fvid_media::clamp_seek_us(50, 40), 40);
        assert_eq!(fvid_media::format_ab_osd(None), "A-B off");
        assert_eq!(fvid_media::format_repeat_osd(fvid_media::RepeatMode::One), "repeat one");
        assert_eq!(fvid_media::format_shuffle_osd(true), "shuffle on");
        assert_eq!(fvid_media::format_subtitle_scale_osd(1_500), "Subtitles 150%");
        assert_eq!(fvid_media::format_jump_osd(90_000_000), "Jump 01:30");
        assert!(fvid_media::format_window_title("demo", 0, 60_000_000, true, 1_500).contains("1.50x"));
        assert_eq!(
            fvid_media::reset_tone_gains(),
            (
                fvid_media::TONE_UNITY_MILLI,
                fvid_media::TONE_UNITY_MILLI,
                fvid_media::TONE_UNITY_MILLI
            )
        );
        assert_eq!(
            fvid_media::tone_gain_step_milli(1_000, fvid_media::TONE_STEP_MILLI),
            1_100
        );
        let mut states = [fvid_media::ToneState::default()];
        let mut frame = [0.25f32];
        fvid_media::apply_tone_frame(&mut frame, &mut states, 2_000, 1_000, 1_000);
        assert!(frame[0] > 0.25);
        assert!(fvid_media::format_tone_osd(1_000, 1_000, 1_000).contains("Tone"));
        let opts = fvid_media::PlayRenderOptions::default();
        let src = vec![0x00_ff_00_00u32, 0x00_00_ff_00, 0x00_00_00_ff, 0x00_ff_ff_00];
        let (w, h, out) = fvid_media::render_play_pixels(2, 2, &src, &opts, None);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out.len(), 4);
        let mut rotated = opts.clone();
        rotated.rotate = fvid_media::RotateMode::Deg90;
        let (rw, rh, _) = fvid_media::render_play_pixels(2, 2, &src, &rotated, None);
        assert_eq!((rw, rh), (2, 2));
        assert_eq!(
            fvid_media::reset_video_adjust(),
            (1_000, 1_000, 1_000, 1_000, 1_000)
        );
        assert_eq!(fvid_media::adjust_step_milli(1_000, 100), 1_100);
        assert_eq!(fvid_media::reset_zoom_pan(), (1_000, 0, 0));
        assert!(fvid_media::format_zoom_osd(2_000).contains("2:1"));
        assert_eq!(
            fvid_media::eq_band_step_milli(1_000, 100),
            fvid_media::clamp_eq_milli(1_100)
        );
        assert_eq!(
            fvid_media::set_eq_gains_from_preset(fvid_media::EqPreset::Flat),
            fvid_media::eq_unity_gains()
        );
        assert!(fvid_media::format_eq_preset_osd(fvid_media::EqPreset::Rock).contains("Rock"));
        assert_eq!(fvid_media::format_pan_osd(10, -5), "Pan 10,-5");
        assert_eq!(
            fvid_media::initial_seek_us("1:30", 200_000_000),
            Some(90_000_000)
        );
        assert_eq!(
            fvid_media::initial_seek_us("1:30", 60_000_000),
            Some(60_000_000)
        );
        assert_eq!(fvid_media::initial_seek_us("bad", 60_000_000), None);
        assert_eq!(fvid_media::gamma_channel(128, 1_000), 128);
        assert!(fvid_media::gamma_channel(128, 2_000) > 128);
        assert_eq!(
            fvid_media::format_audio_channel_osd(fvid_media::AudioChannelMode::Mono),
            "Audio Mono"
        );
        let tmp = std::env::temp_dir().join("fvid-play-controls-playlist");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let a = tmp.join("a.mp4");
        std::fs::write(&a, b"x").unwrap();
        let list = tmp.join("list.m3u");
        std::fs::write(&list, format!("#EXTM3U\n{}\n", a.display())).unwrap();
        let expanded = fvid_media::expand_play_inputs(&[list]).unwrap();
        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].ends_with("a.mp4"));
        assert_eq!(
            fvid_media::initial_stop_us("1:30", 200_000_000),
            Some(90_000_000)
        );
        assert!(fvid_media::should_stop_playback(90_000_001, Some(90_000_000)));
        assert!(!fvid_media::should_stop_playback(10, Some(90_000_000)));
        assert_eq!(fvid_media::rate_from_wheel(1_000, 1), 1_050);
        assert_eq!(fvid_media::rate_from_wheel(250, -1), 250);
        assert_eq!(fvid_media::stop_playback_us(), 0);
        assert_eq!(fvid_media::format_stop_osd(), "Stopped");
        assert!(fvid_media::format_rotate_osd(fvid_media::RotateMode::Deg90).contains("90"));
        assert_eq!(fvid_media::format_flip_osd(true, true), "Flip HV");
        assert_eq!(fvid_media::seek_end_us(100_000_000, Some(50_000_000)), 50_000_000);
        assert_eq!(fvid_media::seek_end_us(100_000_000, None), 100_000_000);
        assert_eq!(
            fvid_media::chapter_index(&[0, 10_000_000, 20_000_000], 15_000_000),
            Some(1)
        );
        assert!(fvid_media::format_chapter_osd(1, 3, 10_000_000).contains("2/3"));
        assert_eq!(fvid_media::format_pause_osd(true), "Paused");
        let mut marks = Vec::new();
        assert!(fvid_media::insert_bookmark(&mut marks, 1_000_000));
        assert_eq!(
            fvid_media::format_bookmark_osd(1_000_000, marks.len(), true),
            "bookmark 00:01 (1)"
        );
        fvid_media::clear_bookmarks(&mut marks);
        assert!(marks.is_empty());
        assert_eq!(fvid_media::format_playlist_osd(0, 3), "1/3");
        let m3u = fvid_media::format_playlist_m3u(&[
            std::path::PathBuf::from("a.mp4"),
            std::path::PathBuf::from("b.mp4"),
        ]);
        assert!(m3u.starts_with("#EXTM3U\n"));
        assert!(m3u.contains("a.mp4\n"));
        assert!(m3u.contains("b.mp4\n"));
        assert_eq!(fvid_media::soft_clip_sample(0.5), 0.5);
        assert!(fvid_media::soft_clip_sample(2.0) < 1.0);
        assert!(fvid_media::soft_clip_sample(-2.0) > -1.0);
        assert_eq!(fvid_media::remaining_media_us(30_000_000, 90_000_000), 60_000_000);
        assert_eq!(
            fvid_media::cycle_position_display(fvid_media::PositionDisplay::Elapsed),
            fvid_media::PositionDisplay::Remaining
        );
        assert!(fvid_media::format_position_osd(
            30_000_000,
            90_000_000,
            fvid_media::PositionDisplay::Remaining
        )
        .starts_with('-'));
        assert!(fvid_media::format_position_osd(
            30_000_000,
            90_000_000,
            fvid_media::PositionDisplay::Both
        )
        .contains("(-"));
        let peak = fvid_media::normalizer_peak_step(0.1, 0.8, 1.0, 0.0);
        assert!((peak - 0.8).abs() < 0.01);
        assert!(fvid_media::normalizer_gain_milli(0.5, 0.95) > 1_000);
        assert!(fvid_media::apply_normalizer_sample(0.5, 2_000).abs() <= 1.0);
        assert!(fvid_media::format_normalizer_osd(true, 1_500).contains("1.50"));
        let mut wide = [0.5f32, -0.5];
        fvid_media::apply_stereo_width(&mut wide, 2_000);
        assert!((wide[0] - 1.0).abs() < 0.01);
        assert!((wide[1] + 1.0).abs() < 0.01);
        let mut mono = [0.5f32, -0.5];
        fvid_media::apply_stereo_width(&mut mono, 0);
        assert!((mono[0] - mono[1]).abs() < 0.01);
        assert_eq!(fvid_media::width_step_milli(1_000, 100), 1_100);
        assert!(fvid_media::format_width_osd(1_500).contains("1.50"));
        assert!((fvid_media::compress_sample(0.2, 0.35, 4.0) - 0.2).abs() < 0.001);
        assert!(fvid_media::compress_sample(0.9, 0.35, 4.0).abs() < 0.9);
        let mut loud = [0.9f32, -0.9];
        fvid_media::apply_compressor(&mut loud, true, 0.35, 4.0);
        assert!(loud[0].abs() < 0.9);
        assert_eq!(fvid_media::format_compressor_osd(true), "Compressor on");
        assert_eq!(fvid_media::cycle_sleep_timer_min(0), 15);
        assert_eq!(fvid_media::cycle_sleep_timer_min(120), 0);
        assert_eq!(fvid_media::sleep_deadline_secs(0, 100), None);
        assert_eq!(fvid_media::sleep_deadline_secs(15, 100), Some(1_000));
        assert!(fvid_media::sleep_timer_fired(Some(100), 100));
        assert!(!fvid_media::sleep_timer_fired(Some(101), 100));
        assert_eq!(fvid_media::format_sleep_osd(30), "Sleep in 30 min");
        assert_eq!(fvid_media::format_sleep_osd(0), "Sleep timer off");
        let mut phones = [1.0f32, 0.0];
        fvid_media::apply_crossfeed(&mut phones, 1_000);
        assert!(phones[0] < 1.0 && phones[1] > 0.0);
        assert_eq!(fvid_media::crossfeed_step_milli(0, 100), 100);
        assert_eq!(fvid_media::format_crossfeed_osd(0), "Crossfeed off");
        assert!(fvid_media::format_crossfeed_osd(500).contains("50%"));
        let mut marks = Vec::new();
        assert!(fvid_media::insert_bookmark(&mut marks, 1_500_000));
        assert!(fvid_media::insert_bookmark(&mut marks, 3_000_000));
        let exported = fvid_media::format_bookmarks_export(&marks);
        assert!(exported.contains("start-time=1.500"));
        assert!(exported.contains("start-time=3.000"));
        let parsed = fvid_media::parse_bookmarks_export(&exported);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].media_us, 1_500_000);
        assert_eq!(parsed[1].media_us, 3_000_000);
        assert_eq!(fvid_media::clamp_fov_milli(10_000), fvid_media::FOV_MIN_MILLI);
        assert_eq!(fvid_media::FOV_DEFAULT_MILLI, 80_000);
        let flat = vec![0x00_ff_00_00u32; 4 * 2];
        let view = fvid_media::project_equirect_view(
            4,
            2,
            &flat,
            2,
            2,
            0,
            0,
            fvid_media::FOV_DEFAULT_MILLI,
        );
        assert_eq!(view.len(), 4);
        assert!(fvid_media::format_spherical_osd(true, 45_000, 10_000, 80_000).contains("360°"));
        assert!(fvid_media::is_hdr_transfer(fvid_media::COLOR_TRC_SMPTE2084));
        assert!(fvid_media::is_hdr_transfer(fvid_media::COLOR_TRC_HLG));
        assert!(!fvid_media::is_hdr_transfer(1));
        assert_eq!(
            fvid_media::cycle_hdr_tonemap(fvid_media::HdrTonemap::Off),
            fvid_media::HdrTonemap::Clip
        );
        assert_eq!(
            fvid_media::auto_hdr_tonemap(fvid_media::COLOR_TRC_SMPTE2084),
            fvid_media::HdrTonemap::Hable
        );
        let mapped = fvid_media::apply_hdr_tonemap_pixel(
            200,
            200,
            200,
            fvid_media::HdrTonemap::Hable,
            fvid_media::COLOR_TRC_SMPTE2084,
        );
        assert!(mapped.0 <= 255);
        assert!(fvid_media::pq_eotf(0.5) > 0.0);
        assert!(fvid_media::hlg_eotf(0.5) > 0.0);
        assert!(fvid_media::pq_eotf(0.8) > fvid_media::pq_eotf(0.2));
        assert!(
            (fvid_media::expand_hdr_channel(0.5, 1) - 1.25).abs() < 0.01
        );
        assert!(fvid_media::format_hdr_tonemap_osd(fvid_media::HdrTonemap::Hable).contains("hable"));
        let sbs = vec![0x00_ff_00_00u32, 0x00_00_ff_00, 0x00_ff_00_00, 0x00_00_ff_00];
        let (aw, ah, anag) = fvid_media::apply_play_stereo3d(
            2,
            2,
            &sbs,
            fvid_media::PlayStereo3D::SbslAnaglyph,
        );
        assert_eq!((aw, ah), (1, 2));
        assert_eq!(anag.len(), 2);
        assert_eq!(
            fvid_media::cycle_play_stereo3d(fvid_media::PlayStereo3D::Off),
            fvid_media::PlayStereo3D::SbslAnaglyph
        );
        assert!(fvid_media::format_play_stereo3d_osd(fvid_media::PlayStereo3D::MonoLeft)
            .contains("mono-left"));
        assert_eq!(
            fvid_media::parse_hdr_tonemap("hable").unwrap(),
            fvid_media::HdrTonemap::Hable
        );
        assert_eq!(
            fvid_media::parse_play_stereo3d("sbsl").unwrap(),
            fvid_media::PlayStereo3D::SbslAnaglyph
        );
        assert_eq!(fvid_media::parse_degrees_milli("45.5").unwrap(), 45_500);
        let mut dual = vec![0u32; 64 * 32];
        for y in 0..32 {
            for x in 0..64 {
                dual[y * 64 + x] = if x < 32 { 0x00_ff_00_00 } else { 0x00_00_00_ff };
            }
        }
        let front = fvid_media::project_equirect_view(
            64,
            32,
            &dual,
            8,
            8,
            0,
            0,
            60_000,
        );
        let back = fvid_media::project_equirect_view(
            64,
            32,
            &dual,
            8,
            8,
            180_000,
            0,
            60_000,
        );
        assert_ne!(front[0], back[0]);
        let gray = vec![0x00_80_80_80u32; 4];
        let mut hdr_opts = fvid_media::PlayRenderOptions::default();
        hdr_opts.hdr_tonemap = fvid_media::HdrTonemap::Hable;
        hdr_opts.color_trc = fvid_media::COLOR_TRC_SMPTE2084;
        let flat = fvid_media::render_play_pixels(2, 2, &gray, &fvid_media::PlayRenderOptions::default(), None);
        let hdr = fvid_media::render_play_pixels(2, 2, &gray, &hdr_opts, None);
        assert_ne!(flat.2[0], hdr.2[0]);
        assert!(fvid_media::format_media_info_osd(
            "demo",
            1920,
            1080,
            60_000_000,
            fvid_media::COLOR_TRC_SMPTE2084,
            true
        )
        .contains("HDR PQ"));
        assert!(fvid_media::format_media_info_osd("demo", 640, 360, -1, 0, true).contains("360°"));
        let jump = fvid_media::random_seek_us(100_000_000, 42).unwrap();
        assert!(jump >= 0 && jump < 100_000_000);
        assert_eq!(fvid_media::random_seek_us(100_000_000, 42), Some(jump));
        assert_eq!(fvid_media::random_seek_us(0, 1), None);
        assert!(fvid_media::detect_equirect_aspect(3840, 1920));
        assert!(!fvid_media::detect_equirect_aspect(1920, 1080));
        assert_eq!(fvid_media::cycle_integer_zoom(1_000), 2_000);
        assert_eq!(fvid_media::cycle_integer_zoom(2_000), 250);
        assert_eq!(fvid_media::fit_window_to_video(640, 360, 1920, 1080), (640, 360));
        assert_eq!(fvid_media::fit_window_to_video(3840, 2160, 1920, 1080), (1920, 1080));
        assert_eq!(fvid_media::subtitle_opacity_u8(1_000), 255);
        assert_eq!(fvid_media::subtitle_opacity_u8(500), 127);
        assert!(fvid_media::format_subtitle_opacity_osd(800).contains("80%"));
        assert!(fvid_media::format_integer_zoom_osd(1_000).contains("1:1"));
        assert_eq!(fvid_media::format_track_osd("Audio", 1, 3), "Audio 2/3");
        assert_eq!(fvid_media::format_track_osd("Subtitles", -1, 2), "Subtitles off");
        assert!(fvid_media::should_quit_at_end(
            true,
            fvid_media::PlaybackContinue::Stop
        ));
        assert!(!fvid_media::should_quit_at_end(
            true,
            fvid_media::PlaybackContinue::Next(1)
        ));
        assert_eq!(fvid_media::clamp_roll_milli(-5_000), 355_000);
        assert_eq!(fvid_media::roll_step_milli(0, fvid_media::ROLL_STEP_MILLI), 5_000);
        assert!(fvid_media::format_spherical_osd_ex(true, 0, 0, 90_000, 80_000).contains("roll"));
        let level = fvid_media::project_equirect_view(64, 32, &dual, 8, 8, 0, 0, 60_000);
        let rolled = fvid_media::project_equirect_view_ex(64, 32, &dual, 8, 8, 0, 0, 90_000, 60_000);
        assert!(level.iter().zip(rolled.iter()).any(|(a, b)| a != b));
        assert_eq!(
            fvid_media::cycle_subtitle_position(fvid_media::SubtitlePosition::Bottom),
            fvid_media::SubtitlePosition::Center
        );
        assert_eq!(
            fvid_media::subtitle_block_top_y(100.0, 2, 10.0, 8.0, fvid_media::SubtitlePosition::Top),
            8.0
        );
        assert!(
            fvid_media::subtitle_block_top_y(
                100.0,
                2,
                10.0,
                8.0,
                fvid_media::SubtitlePosition::Bottom
            ) > 50.0
        );
        assert!(fvid_media::format_subtitle_position_osd(fvid_media::SubtitlePosition::Center)
            .contains("center"));
        assert!(!fvid_media::osd_should_clear(100, fvid_media::OSD_TIMEOUT_DEFAULT_MS));
        assert!(fvid_media::osd_should_clear(3_000, fvid_media::OSD_TIMEOUT_DEFAULT_MS));
        assert_eq!(
            fvid_media::clamp_osd_timeout_ms(10),
            fvid_media::OSD_TIMEOUT_MIN_MS
        );
        assert!(fvid_media::mouse_should_hide(1_000, fvid_media::MOUSE_HIDE_DEFAULT_MS));
        assert!(!fvid_media::mouse_should_hide(10, fvid_media::MOUSE_HIDE_DEFAULT_MS));
        assert_eq!(fvid_media::audio_peak_milli(&[0.0, 0.5, -0.25]), 500);
        assert_eq!(fvid_media::format_vu_osd(500), "VU 50%");
        let snap = fvid_media::snapshot_path_in_dir(
            Some(Path::new("shots")),
            Path::new("clips/demo.mp4"),
            3,
            "png",
        );
        assert_eq!(snap, PathBuf::from("shots/demo-fvid-3.png"));
        assert!(fvid_media::format_snapshot_dir_osd(Some(Path::new("shots"))).contains("shots"));
        assert_eq!(
            fvid_media::clamp_network_cache_ms(120_000),
            fvid_media::NETWORK_CACHE_MAX_MS
        );
        assert!(fvid_media::format_network_cache_osd(1_000).contains("1000"));
        assert!(fvid_media::format_hotkeys_help_osd().contains("Space"));
    }
}
