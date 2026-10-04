//! Bounded GPU input ownership for native movie encoding, without libav.
use crate::owned_nvdec_movie::AvcMovieRenderer;
use fvid_cuda::{
    CodecDevice, Nv12Buffer, Nv12Processor, Nv12Transform, NvencCodec, NvencPacket, NvencSession,
    NvencSubmit,
};
use std::{
    collections::VecDeque,
    io::{Read, Seek},
    mem::ManuallyDrop,
    time::{Duration, Instant},
};

struct Pool {
    count: usize,
    pending: VecDeque<usize>,
}
impl Pool {
    fn new(count: usize) -> Result<Self, String> {
        if !(1..=64).contains(&count) {
            return Err("movie encoder pool requires 1..=64 slots".into());
        }
        Ok(Self {
            count,
            pending: VecDeque::new(),
        })
    }
    fn available(&self) -> Option<usize> {
        (0..self.count).find(|slot| !self.pending.contains(slot))
    }
    fn accepted(&mut self, slot: usize) -> Result<(), String> {
        if slot >= self.count || self.pending.contains(&slot) {
            return Err("movie encoder slot is still pending".into());
        }
        self.pending.push_back(slot);
        Ok(())
    }
    fn received(&mut self) -> Result<usize, String> {
        self.pending
            .pop_front()
            .ok_or_else(|| "unexpected movie encoder output".into())
    }
}
/// Encodes movie occurrences with their original track-clock timestamps.
/// Each accepted input retains its own GPU allocation until output is drained;
/// rendering the next event never overwrites an in-flight NVENC input.
pub struct AvcMovieEncoder<R: Read + Seek> {
    renderer: ManuallyDrop<AvcMovieRenderer<R>>,
    encoder: ManuallyDrop<NvencSession>,
    copy: ManuallyDrop<Nv12Processor>,
    buffers: ManuallyDrop<Vec<Nv12Buffer>>,
    handles: Vec<(usize, usize)>,
    pool: Pool,
    eos: bool,
    failed: bool,
    wait: Duration,
    codec: NvencCodec,
}
impl<R: Read + Seek> AvcMovieEncoder<R> {
    pub fn new(
        renderer: AvcMovieRenderer<R>,
        codec: NvencCodec,
        fps_num: u32,
        fps_den: u32,
        slots: usize,
        wait: Duration,
    ) -> Result<Self, String> {
        let pool = Pool::new(slots)?;
        if wait.is_zero() {
            return Err("movie encoder wait must be positive".into());
        }
        let ordinal = renderer.ordinal();
        let encoder = NvencSession::open(CodecDevice::new(ordinal)?)?;
        let copy = Nv12Processor::new(ordinal)?;
        let mut this = Self {
            renderer: ManuallyDrop::new(renderer),
            encoder: ManuallyDrop::new(encoder),
            copy: ManuallyDrop::new(copy),
            buffers: ManuallyDrop::new(Vec::new()),
            handles: Vec::new(),
            pool,
            eos: false,
            failed: false,
            wait,
            codec,
        };
        let (width, height) = this.renderer.buffer().dimensions();
        this.encoder
            .initialize_nv12(codec, width, height, fps_num, fps_den)?;
        for _ in 0..slots {
            this.buffers.push(Nv12Buffer::new(ordinal, width, height)?);
            let buffer = this.buffers.last().unwrap();
            let view = buffer.view()?;
            // SAFETY: Allocation uses the same primary context and is retained
            // until encoder close succeeds, including construction failures.
            let input = unsafe {
                this.encoder
                    .register_nv12(view.y, view.pitch_y, buffer.byte_len() as u64)
            }?;
            let output = this.encoder.create_output()?;
            this.handles.push((input, output));
        }
        Ok(this)
    }
    pub fn media_timescale(&self) -> u32 {
        self.renderer.media_timescale()
    }
    pub fn next_packet(&mut self) -> Result<Option<NvencPacket>, String> {
        if self.failed {
            return Err("native movie encoder failed; reopen it".into());
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_inner(&mut self) -> Result<Option<NvencPacket>, String> {
        let deadline = Instant::now()
            .checked_add(self.wait)
            .ok_or("movie encoder wait overflow")?;
        loop {
            if let Some(packet) = self.encoder.receive()? {
                self.pool.received()?;
                return Ok(Some(packet));
            }
            if self.eos && self.pool.pending.is_empty() {
                return Ok(None);
            }
            if !self.eos
                && let Some(slot) = self.pool.available()
            {
                if let Some(event) = self.renderer.render_next()? {
                    let timestamp =
                        u64::try_from(event.start).map_err(|_| "negative movie timestamp")?;
                    let duration = u64::try_from(
                        event
                            .end
                            .checked_sub(event.start)
                            .ok_or("movie duration overflow")?,
                    )
                    .map_err(|_| "negative movie duration")?;
                    let buffer = &self.buffers[slot];
                    let (width, height) = buffer.dimensions();
                    self.copy.follow_stream(buffer.stream_handle()?);
                    let copied = self.copy.apply(
                        self.renderer.buffer().view()?,
                        buffer.view()?,
                        Nv12Transform {
                            out_width: width,
                            out_height: height,
                            ..Default::default()
                        },
                    );
                    self.copy.synchronize()?;
                    buffer.synchronize()?;
                    copied?;
                    let (input, output) = self.handles[slot];
                    loop {
                        match self
                            .encoder
                            .submit_nv12(input, output, timestamp, duration)?
                        {
                            NvencSubmit::Busy => wait_until(deadline)?,
                            NvencSubmit::Ready | NvencSubmit::Queued => {
                                self.pool.accepted(slot)?;
                                break;
                            }
                        }
                    }
                    continue;
                }
                self.encoder.finish()?;
                self.eos = true;
                continue;
            }
            wait_until(deadline)?;
        }
    }
    /// Write AVC output with the owned Matroska writer. This is video-only;
    /// audio remux/encode and atomic file publication belong to the caller.
    pub fn write_avc_matroska<W: std::io::Write + Seek>(
        &mut self,
        output: &mut W,
        max_packet_bytes: usize,
    ) -> Result<u64, String> {
        if self.codec != NvencCodec::H264 {
            return Err("AVC Matroska export requires H.264 encoding".into());
        }
        let result = self.write_avc_inner(output, max_packet_bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn write_avc_inner<W: std::io::Write + Seek>(
        &mut self,
        output: &mut W,
        max_packet_bytes: usize,
    ) -> Result<u64, String> {
        let first = self
            .next_packet()?
            .ok_or("movie encoder returned no video")?;
        let first_sample = crate::owned_avc_annexb::convert(&first.bytes, max_packet_bytes)?;
        let configuration = first_sample
            .configuration
            .as_ref()
            .ok_or("first AVC output lacks SPS/PPS")?;
        let (width, height) = self.renderer.buffer().dimensions();
        let tracks = [crate::owned_matroska::TrackSpec {
            encoding: crate::owned_matroska::Encoding::Avc {
                configuration,
                width,
                height,
            },
            name: "",
            language: "und",
        }];
        let mut writer =
            crate::owned_matroska::PacketWriter::new(output, &tracks).map_err(|e| e.to_string())?;
        let scale = self.media_timescale();
        writer
            .write_packet(
                0,
                ticks_ns(first.timestamp, scale)?,
                ticks_ns(first.duration, scale)?,
                first_sample.sync,
                &first_sample.sample,
            )
            .map_err(|e| e.to_string())?;
        let mut count = 1u64;
        while let Some(packet) = self.next_packet()? {
            let sample = crate::owned_avc_annexb::convert(&packet.bytes, max_packet_bytes)?;
            if sample
                .configuration
                .as_ref()
                .is_some_and(|config| config != configuration)
            {
                return Err("AVC configuration changed during movie export".into());
            }
            writer
                .write_packet(
                    0,
                    ticks_ns(packet.timestamp, scale)?,
                    ticks_ns(packet.duration, scale)?,
                    sample.sync,
                    &sample.sample,
                )
                .map_err(|e| e.to_string())?;
            count = count.checked_add(1).ok_or("movie packet count overflow")?;
        }
        writer.finish().map_err(|e| e.to_string())?;
        Ok(count)
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.failed = true;
        self.copy.synchronize()?;
        self.encoder.close()?;
        self.renderer.close()
    }
}
impl<R: Read + Seek> Drop for AvcMovieEncoder<R> {
    fn drop(&mut self) {
        // Keep registered allocations alive if driver cleanup cannot finish.
        if self.close().is_err() {
            return;
        }
        unsafe {
            ManuallyDrop::drop(&mut self.encoder);
            ManuallyDrop::drop(&mut self.copy);
            ManuallyDrop::drop(&mut self.buffers);
            ManuallyDrop::drop(&mut self.renderer);
        }
    }
}
fn ticks_ns(ticks: u64, timescale: u32) -> Result<u64, String> {
    if timescale == 0 {
        return Err("movie timescale is zero".into());
    }
    u64::try_from(u128::from(ticks) * 1_000_000_000 / u128::from(timescale))
        .map_err(|_| "movie nanosecond timestamp overflow".into())
}

fn wait_until(deadline: Instant) -> Result<(), String> {
    if Instant::now() >= deadline {
        return Err("native movie encoder timed out; pool may be smaller than driver delay".into());
    }
    std::thread::sleep(Duration::from_millis(1));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn movie_clock_conversion_is_checked() {
        assert_eq!(ticks_ns(1, 3).unwrap(), 333_333_333);
        assert_eq!(ticks_ns(6, 1).unwrap(), 6_000_000_000);
        assert!(ticks_ns(1, 0).is_err());
        assert!(ticks_ns(u64::MAX, 1).is_err());
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA NVDEC and NVENC"]
    fn synthetic_movie_encodes_and_muxes_without_libav() {
        use crate::owned_nvdec_movie::AvcMovieReader;
        use crate::owned_nvdec_mp4::AvcMp4Input;
        use std::io::Cursor;
        let source = AvcMp4Input::open(
            Cursor::new(
                include_bytes!("../../../tests/fixtures/playback-errors/edit-empty-spans.mov")
                    .as_slice(),
            ),
            Default::default(),
        )
        .unwrap();
        let expected = source.movie_presentations(1000).unwrap();
        let scale = source.track().timescale;
        let (width, height) = source.coded_dimensions();
        let reader = AvcMovieReader::new(source, 0, 32, 2, 1000, 8).unwrap();
        let renderer = AvcMovieRenderer::new(
            reader,
            Nv12Transform {
                out_width: width,
                out_height: height,
                hflip: true,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let mut encoder = AvcMovieEncoder::new(
            renderer,
            NvencCodec::H264,
            60,
            1,
            32,
            Duration::from_secs(10),
        )
        .unwrap();
        let mut output = Cursor::new(Vec::new());
        assert_eq!(
            encoder.write_avc_matroska(&mut output, 1 << 20).unwrap(),
            expected.len() as u64
        );
        encoder.close().unwrap();
        let mut saved = crate::owned_webm::WebmReader::open(
            Cursor::new(output.into_inner()),
            Default::default(),
        )
        .unwrap();
        let mut expected: Vec<_> = expected
            .iter()
            .map(|event| {
                (
                    ticks_ns(event.start as u64, scale).unwrap(),
                    ticks_ns((event.end - event.start) as u64, scale).unwrap(),
                )
            })
            .collect();
        expected.sort_unstable();
        let mut actual: Vec<_> = saved
            .packets
            .iter()
            .map(|packet| {
                (
                    u64::try_from(packet.pts_ns).unwrap(),
                    packet.duration_ns.unwrap(),
                )
            })
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        let mut software = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
            &saved.tracks[0].codec_private,
            16 << 20,
        )
        .unwrap();
        for index in 0..saved.packets.len() {
            let packet = saved.read_packet(index).unwrap();
            let picture = software.decode_order(&packet).unwrap().unwrap();
            assert_eq!(picture.dimensions(), (width as usize, height as usize));
        }
    }
    #[test]
    fn queued_inputs_are_never_reused_before_output() {
        let mut pool = Pool::new(2).unwrap();
        assert_eq!(pool.available(), Some(0));
        pool.accepted(0).unwrap();
        pool.accepted(1).unwrap();
        assert_eq!(pool.available(), None);
        assert!(pool.accepted(0).is_err());
        assert_eq!(pool.received().unwrap(), 0);
        assert_eq!(pool.available(), Some(0));
        pool.accepted(0).unwrap();
        assert_eq!(pool.received().unwrap(), 1);
        assert_eq!(pool.received().unwrap(), 0);
        assert!(pool.received().is_err());
        assert!(Pool::new(0).is_err());
        assert!(Pool::new(65).is_err());
    }
}
