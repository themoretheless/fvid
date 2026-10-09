/// Per-block presentation controls. Invisible video blocks still establish
/// decoder references but do not extend the presentation duration.
#[derive(Clone, Copy, Debug, Default)]
pub struct PacketOptions {
    pub discard_padding_ns: i64,
    pub invisible: bool,
}

/// Streaming packet writer. Feed each track in decode order, interleaving tracks
/// at the call site. PTS can move backwards for B-frames; each packet has its own
/// nanosecond-clock cluster, avoiding signed 16-bit block timestamp overflow.
/// No payload copies or accumulated packet index. Discard output on any error.
pub struct PacketWriter<'a, W> {
    output: &'a mut W,
    segment_size: u64,
    duration_offset: u64,
    tracks_offset: u64,
    tracks_bytes: usize,
    written: Vec<bool>,
    delays: Vec<u64>,
    pcm: Vec<Option<(u32, u16)>>,
    end_ns: u64,
    event: ProgressEvent,
    failed: bool,
}
impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    fn new_prepared(
        output: &'a mut W,
        entries: &[u8],
        file_elements: &[u8],
        delays: Vec<u64>,
        pcm: Vec<Option<(u32, u16)>>,
    ) -> Result<Self> {
        if delays.is_empty() || delays.len() > 126 || delays.len() != pcm.len() {
            return Err(invalid("Matroska requires consistent 1..=126 tracks"));
        }
        if output.stream_position()? != 0 {
            return Err(invalid("Matroska output must start at zero"));
        }
        let ebml = [
            uint(0x4286, 1)?,
            uint(0x42f7, 1)?,
            uint(0x42f2, 4)?,
            uint(0x42f3, 8)?,
            element(0x4282, b"matroska")?,
            uint(0x4287, 4)?,
            uint(0x4285, 2)?,
        ]
        .concat();
        output.write_all(&element(0x1a45dfa3, &ebml)?)?;
        output.write_all(&0x18538067u32.to_be_bytes())?;
        let segment_size = output.stream_position()?;
        output.write_all(&[1, 255, 255, 255, 255, 255, 255, 255])?;
        let info = element(
            0x1549a966,
            &[
                uint(0x2ad7b1, 1)?,
                element(0x4d80, b"FVid")?,
                element(0x5741, b"FVid")?,
                element(0x4489, &0f64.to_be_bytes())?,
            ]
            .concat(),
        )?;
        let duration_offset = output.stream_position()? + info.len() as u64 - 8;
        output.write_all(&info)?;
        let tracks = element(0x1654ae6b, entries)?;
        let tracks_offset = output.stream_position()?;
        let tracks_bytes = tracks.len();
        output.write_all(&tracks)?;
        output.write_all(file_elements)?;
        Ok(Self {
            output,
            segment_size,
            duration_offset,
            tracks_offset,
            tracks_bytes,
            written: vec![false; delays.len()],
            delays,
            pcm,
            end_ns: 0,
            event: ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
            failed: false,
        })
    }
    // Only the private single-track ADTS adapter uses this fixed-size rewrite.
    // Payload timestamps remain in nanoseconds, independent of output rate.
    fn rewrite_adts_rate(&mut self, asc: &[u8], rate: u32, channels: u16) -> Result<()> {
        let spec = TrackSpec { encoding: Encoding::Aac { configuration: asc, sample_rate: rate, channels }, name: "", language: "" };
        let tracks = element(0x1654ae6b, &track_entry(&spec, 1, None)?)?;
        if self.written.len() != 1 || tracks.len() != self.tracks_bytes {
            return Err(invalid("ADTS track rewrite changed header size"));
        }
        let end = self.output.stream_position()?;
        self.output.seek(SeekFrom::Start(self.tracks_offset))?;
        self.output.write_all(&tracks)?;
        self.output.seek(SeekFrom::Start(end))?;
        Ok(())
    }
    pub fn event(&self) -> ProgressEvent {
        self.event
    }
    /// Flush pending container bytes without finalizing or publishing output.
    pub fn flush(&mut self) -> Result<()> {
        if self.failed {return Err(invalid("Matroska writer failed"));}
        if let Err(error) = self.output.flush() {self.failed=true;return Err(error.into());}
        Ok(())
    }

    /// `track` is zero-based. Durations/PTS are in nanoseconds. The sync flag
    /// states independent decodability, not whether PTS follows the last packet.
    pub fn write_packet(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
    ) -> Result<()> {
        self.write_packet_with_padding(track, pts_ns, duration_ns, sync, payload, 0)
    }
    /// Positive padding discards the end; negative padding discards the start.
    /// Nanoseconds are independent of the segment timestamp scale.
    pub fn write_packet_with_padding(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        discard_padding_ns: i64,
    ) -> Result<()> {
        self.write_packet_with_options(
            track,
            pts_ns,
            duration_ns,
            sync,
            payload,
            PacketOptions {
                discard_padding_ns,
                invisible: false,
            },
        )
    }
    /// Write a packet with explicit decode-only or padding controls.
    pub fn write_packet_with_options(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        options: PacketOptions,
    ) -> Result<()> {
        if self.failed {
            return Err(invalid("Matroska writer failed"));
        }
        let result = self.packet(track, pts_ns, duration_ns, sync, payload, options);
        self.failed = result.is_err();
        result
    }
    fn packet(
        &mut self,
        track: usize,
        pts_ns: u64,
        duration_ns: u64,
        sync: bool,
        payload: &[u8],
        options: PacketOptions,
    ) -> Result<()> {
        let discard_padding_ns = options.discard_padding_ns;
        if track >= self.written.len() || duration_ns == 0 || payload.is_empty() {
            return Err(invalid("invalid Matroska packet"));
        }
        if let Some((rate, channels)) = self.pcm[track] {
            let frame_bytes = usize::from(channels) * 4;
            if !payload.len().is_multiple_of(frame_bytes) || options.invisible || !sync
                || payload.as_chunks::<4>().0.iter().any(|p|!f32::from_le_bytes(*p).is_finite()) {
                return Err(invalid("invalid Matroska float PCM packet"));
            }
            let frames = (payload.len() / frame_bytes) as u128;
            let span = frames * 1_000_000_000;
            if u128::from(duration_ns) < span / u128::from(rate)
                || u128::from(duration_ns) > span.div_ceil(u128::from(rate)) {
                return Err(invalid("Matroska PCM duration disagrees with sample count"));
            }
        }
        if discard_padding_ns.unsigned_abs() > duration_ns {
            return Err(invalid("Matroska padding exceeds packet duration"));
        }
        let end = pts_ns
            .checked_add(duration_ns)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or_else(|| invalid("Matroska timestamp overflow"))?;
        let packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let bytes = self
            .event
            .payload_bytes
            .checked_add(payload.len() as u64)
            .ok_or_else(|| invalid("payload count overflow"))?;
        let timestamp = uint(0xe7, pts_ns)?;
        let duration = uint(0x9b, duration_ns)?;
        // ReferenceBlock=0 is the specified marker for dependent blocks whose
        // precise reference graph is unknown to the container-only writer.
        let reference = if sync {
            Vec::new()
        } else {
            element(0xfb, &[0])?
        };
        let padding = if discard_padding_ns == 0 {
            Vec::new()
        } else {
            element(0x75a2, &discard_padding_ns.to_be_bytes())?
        };
        let block = 4 + payload.len() as u64;
        let group = 1
            + size(block)?.len() as u64
            + block
            + duration.len() as u64
            + reference.len() as u64
            + padding.len() as u64;
        let cluster = timestamp.len() as u64 + 1 + size(group)?.len() as u64 + group;
        head(self.output, 0x1f43b675, cluster)?;
        self.output.write_all(&timestamp)?;
        head(self.output, 0xa0, group)?;
        head(self.output, 0xa1, block)?;
        self.output.write_all(&[
            0x80 | (track as u8 + 1),
            0,
            0,
            if options.invisible { 0x08 } else { 0 },
        ])?;
        self.output.write_all(payload)?;
        self.output.write_all(&duration)?;
        self.output.write_all(&reference)?;
        self.output.write_all(&padding)?;
        self.written[track] = true;
        let presented_end = end
            .saturating_sub(self.delays[track])
            .saturating_sub(discard_padding_ns.max(0) as u64);
        if !options.invisible {
            self.end_ns = self.end_ns.max(presented_end);
        }
        self.event.packets = packets;
        self.event.payload_bytes = bytes;
        Ok(())
    }
    /// Patch finite segment size and presentation duration. `done` remains false:
    /// the caller owns flushing, syncing and atomic publication.
    pub fn finish(self) -> Result<ProgressEvent> {
        self.finish_inner(false)
    }
    /// Finalize an intentionally bounded prefix, retaining declared empty tracks.
    /// Failed writes still prevent finalization; a prefix with no presented duration omits Duration.
    pub fn finish_prefix(self) -> Result<ProgressEvent> {
        self.finish_inner(true)
    }
    fn finish_inner(self, prefix: bool) -> Result<ProgressEvent> {
        if self.failed || (!prefix && self.written.iter().any(|v| !v)) {
            return Err(invalid("incomplete Matroska tracks"));
        }
        if !prefix && self.end_ns == 0 {
            return Err(invalid("Matroska has no presentation duration"));
        }
        let end = self.output.stream_position()?;
        let length = end - self.segment_size - 8;
        if length >= (1u64 << 56) - 1 {
            return Err(invalid("Matroska segment exceeds size range"));
        }
        self.output.seek(SeekFrom::Start(self.segment_size))?;
        self.output
            .write_all(&(length | (1u64 << 56)).to_be_bytes())?;
        if self.end_ns == 0 {
            // Replace the complete 11-byte Duration element with equally sized Void.
            self.output.seek(SeekFrom::Start(self.duration_offset - 3))?;
            self.output.write_all(&[0xec, 0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
        } else {
            self.output.seek(SeekFrom::Start(self.duration_offset))?;
            self.output.write_all(&(self.end_ns as f64).to_be_bytes())?;
        }
        self.output.seek(SeekFrom::Start(end))?;
        Ok(self.event)
    }
}
