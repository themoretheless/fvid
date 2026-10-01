//! Y4M input validation, transform planning, and bounded streaming execution.
#[cfg(any(feature = "gpu", feature = "cuda"))]
use crate::resident;
use crate::{Backend, ExecutionOptions, FrameView, Result, backend, buffer, invalid};
use std::io::{BufRead, Write};

const MAX_LINE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Yuv420,
    Yuv422,
    Yuv444,
}
impl PixelFormat {
    pub(crate) fn subsampling(self) -> (usize, usize) {
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
    pub(crate) tokens: Vec<String>,
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
pub(crate) struct Plane {
    pub(crate) input_offset: usize,
    pub(crate) output_offset: usize,
    pub(crate) stride: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}
/// A validated plan. Crop coordinates refer to the input, before either reflection.
#[derive(Debug)]
pub struct Plan {
    pub(crate) planes: Vec<Plane>,
    pub(crate) input_len: usize,
    pub(crate) output_len: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) horizontal: bool,
    pub(crate) vertical: bool,
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
pub(crate) fn line<R: BufRead>(reader: &mut R, bytes: &mut Vec<u8>) -> Result<bool> {
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
    reader: R,
    writer: W,
    transforms: &[Transform],
    memory_limit: usize,
    options: ExecutionOptions,
) -> Result<(Stats, resident::TransferStats)> {
    let stages: Vec<_> = transforms
        .iter()
        .copied()
        .map(resident::GpuStage::from)
        .collect();
    process_gpu_stages(reader, writer, &stages, memory_limit, options)
}

/// Programmable GPU stages with one upload/download per frame.
#[cfg(any(feature = "gpu", feature = "cuda"))]
pub fn process_gpu_stages<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    stages: &[resident::GpuStage],
    memory_limit: usize,
    options: ExecutionOptions,
) -> Result<(Stats, resident::TransferStats)> {
    let mut marker = Vec::new();
    if !line(&mut reader, &mut marker)? {
        return Err(invalid("empty input"));
    }
    let header = Header::parse(&marker)?;
    let mut pipeline = resident::GpuPipeline::with_stages(&header, stages, options, memory_limit)?;
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
