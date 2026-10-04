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
        let colour = this
            .renderer
            .metadata()
            .options
            .video
            .and_then(|video| video.colour)
            .map(|c| fvid_cuda::NvencColour {
                primaries: if c.primaries == 0 { 2 } else { c.primaries },
                transfer: if c.transfer == 0 { 2 } else { c.transfer },
                matrix: if c.matrix == 0 { 2 } else { c.matrix },
                full_range: c.full_range,
            });
        this.encoder
            .initialize_nv12_with_colour(codec, width, height, fps_num, fps_den, colour)?;
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
    pub fn device_filter_passes(&self) -> u64 {
        self.renderer.device_filter_passes()
    }
    pub fn media_timescale(&self) -> u32 {
        self.renderer.media_timescale()
    }
    pub fn next_packet(&mut self) -> Result<Option<NvencPacket>, String> {
        if self.failed {
            return Err("native movie encoder failed; reopen it".into());
        }
        self.next_packet_controlled(None)
    }
    fn next_packet_controlled(
        &mut self,
        cancel: Option<&fvid_control::CancelFlag>,
    ) -> Result<Option<NvencPacket>, String> {
        if self.failed {
            return Err("native movie encoder failed; reopen it".into());
        }
        let result = self.next_inner(cancel);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_inner(
        &mut self,
        cancel: Option<&fvid_control::CancelFlag>,
    ) -> Result<Option<NvencPacket>, String> {
        let deadline = Instant::now()
            .checked_add(self.wait)
            .ok_or("movie encoder wait overflow")?;
        loop {
            check_cancel(cancel)?;
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
                            NvencSubmit::Busy => wait_until(deadline, cancel)?,
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
            wait_until(deadline, cancel)?;
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
        let result = self
            .write_avc_inner(output, max_packet_bytes, None, None)
            .map(|event| event.packets);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn write_avc_inner<W: std::io::Write + Seek>(
        &mut self,
        output: &mut W,
        max_packet_bytes: usize,
        cancel: Option<&fvid_control::CancelFlag>,
        progress: Option<&fvid_control::ProgressHook>,
    ) -> Result<fvid_control::ProgressEvent, String> {
        check_cancel(cancel)?;
        let first = self
            .next_packet_controlled(cancel)?
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
            name: &self.renderer.metadata().name,
            language: if self.renderer.metadata().language.is_empty() {
                "und"
            } else {
                &self.renderer.metadata().language
            },
        }];
        let mut writer = crate::owned_matroska::PacketWriter::new_with_metadata(
            output,
            &tracks,
            &[self.renderer.metadata().options],
            &self.renderer.metadata().file,
        )
        .map_err(|e| e.to_string())?;
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
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
        while let Some(packet) = self.next_packet_controlled(cancel)? {
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
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
        }
        check_cancel(cancel)?;
        writer.finish().map_err(|e| e.to_string())
    }
    /// Finalize and close the encoder before atomically publishing a new .mkv.
    /// Existing output is preserved. Progress is done only after publication.
    pub fn export_avc_matroska(
        &mut self,
        destination: &std::path::Path,
        max_packet_bytes: usize,
        cancel: Option<&fvid_control::CancelFlag>,
        progress: Option<&fvid_control::ProgressHook>,
    ) -> Result<fvid_media_info::DecodeStats, String> {
        if self.codec != NvencCodec::H264 {
            return Err("AVC Matroska export requires H.264 encoding".into());
        }
        let (width, height) = self.renderer.buffer().dimensions();
        let result = crate::owned_matroska::export_atomic(destination, cancel, progress, |file| {
            let event = self
                .write_avc_inner(file, max_packet_bytes, cancel, progress)
                .map_err(crate::owned_matroska::Error)?;
            self.close().map_err(crate::owned_matroska::Error)?;
            Ok((
                fvid_media_info::DecodeStats {
                    backend: "owned-cuda-nvdec-nvenc",
                    video_frames: event.packets,
                    width,
                    height,
                    pixel_format: "nv12".into(),
                    decode_errors: 0,
                },
                event,
                0,
            ))
        })
        .map_err(|e| e.to_string());
        if result.is_err() {
            self.failed = true;
        }
        result.map(|(stats, _, _)| stats)
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

fn check_cancel(cancel: Option<&fvid_control::CancelFlag>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.is_cancelled()) {
        return Err("cancelled".into());
    }
    Ok(())
}
fn wait_until(deadline: Instant, cancel: Option<&fvid_control::CancelFlag>) -> Result<(), String> {
    check_cancel(cancel)?;
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
    fn cancellation_interrupts_waits_before_timeout() {
        let flag = fvid_control::CancelFlag::new();
        flag.cancel();
        assert_eq!(
            wait_until(Instant::now() + Duration::from_secs(10), Some(&flag)).unwrap_err(),
            "cancelled"
        );
        assert!(check_cancel(None).is_ok());
    }
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
        const EMPTY: &[u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/edit-empty-spans.mov");
        const METADATA: &[u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/avc-cuda-video-metadata.mp4");
        for (bytes, white_shader, clipped) in [
            (EMPTY, false, false),
            (METADATA, false, false),
            (EMPTY, true, false),
            (EMPTY, false, true),
        ] {
            let source = AvcMp4Input::open(Cursor::new(bytes), Default::default()).unwrap();
            let unit = (1000 * u64::from(source.track().timescale))
                .div_ceil(u64::from(source.movie_timescale())) as i64;
            let interval = clipped.then_some((unit / 2, unit * 9 / 2));
            let expected = crate::owned_nvdec_movie::clip_presentations(
                source.movie_presentations(1000).unwrap(),
                interval,
            )
            .unwrap();
            let scale = source.track().timescale;
            let (width, height) = source.coded_dimensions();
            let reader =
                AvcMovieReader::new_with_interval(source, 0, 32, 2, 1000, 8, interval).unwrap();
            let shader = white_shader.then(|| fvid_cuda::ByteShader::new(
                "__device__ unsigned int process_byte(unsigned int value, unsigned int plane, unsigned int x, unsigned int y) { return plane == 0u ? 235u : 128u; }").unwrap());
            let renderer = AvcMovieRenderer::new_with_shader(
                reader,
                Nv12Transform {
                    out_width: width,
                    out_height: height,
                    hflip: true,
                    ..Default::default()
                },
                false,
                shader.as_ref(),
            )
            .unwrap();
            let metadata = renderer.metadata().clone();
            let mut encoder = AvcMovieEncoder::new(
                renderer,
                NvencCodec::H264,
                60,
                1,
                32,
                Duration::from_secs(10),
            )
            .unwrap();
            struct Directory(std::path::PathBuf);
            impl Drop for Directory {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let directory = Directory(std::env::temp_dir().join(format!(
                    "fvid-native-export-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )));
            std::fs::create_dir(&directory.0).unwrap();
            let output = directory.0.join("output.mkv");
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = events.clone();
            let published = output.clone();
            let progress = fvid_control::ProgressHook::new(move |event| {
                if event.done {
                    assert!(published.exists());
                }
                observed.lock().unwrap().push(event);
            });
            let stats = encoder
                .export_avc_matroska(&output, 1 << 20, None, Some(&progress))
                .unwrap();
            assert_eq!(stats.video_frames, expected.len() as u64);
            let passes = if white_shader {
                expected.len()
            } else {
                expected
                    .iter()
                    .filter(|event| event.sample.is_some())
                    .count()
            };
            assert_eq!(encoder.device_filter_passes(), passes as u64);
            let events = events.lock().unwrap();
            assert!(events.last().unwrap().done);
            assert!(events[..events.len() - 1].iter().all(|event| !event.done));
            let before = std::fs::read(&output).unwrap();
            assert!(
                encoder
                    .export_avc_matroska(&output, 1 << 20, None, None)
                    .is_err()
            );
            assert_eq!(std::fs::read(&output).unwrap(), before);
            assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
            let mut saved =
                crate::owned_webm::WebmReader::open(Cursor::new(before), Default::default())
                    .unwrap();
            assert_eq!(saved.tags, metadata.file.tags);
            assert_eq!(saved.chapters.len(), metadata.file.chapters.len());
            assert_eq!(saved.tracks[0].rotation, metadata.options.rotation);
            assert_eq!(
                Some(saved.tracks[0].colour),
                metadata.options.video.unwrap().colour
            );
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
                if white_shader {
                    // Includes the two empty spans: ignoring their shader would
                    // produce limited black Y=16 instead of this uniform white.
                    assert!(picture.y.iter().all(|sample| sample.abs_diff(235) <= 2));
                    assert!(
                        picture
                            .cb
                            .iter()
                            .chain(&picture.cr)
                            .all(|sample| sample.abs_diff(128) <= 2)
                    );
                }
            }
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
