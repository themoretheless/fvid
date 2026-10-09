
/// What a limit guards: how much the reader may take in, and how many frames it
/// may agree to list.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most frames one stream may hold.
    pub packets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            packets: 1 << 20,
        }
    }
}

/// The tag `make_audio_decoder` answers for AAC. It is the MP4
/// fourcc rather than the Matroska `A_AAC` because the dispatch has no arm for
/// that one, and because the coding's setup block is an MP4 `esds`, which is
/// what this reader hands over with it.
pub const TAG: &str = "mp4a";

/// The rates an ADTS sampling frequency index names, in table order. Index 15 is
/// reserved here, where the AudioSpecificConfig instead spells a rate in 24
/// bits, so a header carrying 13, 14 or 15 states no rate this reader can trust.
const FREQUENCIES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// What an ADTS header states about itself: where its audio starts, how long the
/// frame is, and the coding and geometry to read out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub sample_rate: u32,
    pub channels: u16,
    /// Raw AAC blocks multiplexed in this transport frame (one to four).
    pub raw_blocks: u8,
    /// Bytes before the first raw block: seven without CRC; 7 + 2*raw_blocks
    /// with CRC (including positions and the header check).
    pub header_bytes: usize,
    /// Total frame length, header and CRC included, as the frame states it.
    pub frame_bytes: usize,
    /// Fixed-header setup; configuration=0 needs the PCE-aware reader accessor.
    /// The two-byte AudioSpecificConfig this frame's coding, rate and layout
    /// spell, which is what the decoder's setup block carries.
    pub asc: [u8; 2],
}

/// Read just the fixed header of the ADTS frame that starts at `bytes`.
///
/// `None` for a run of bytes no ADTS frame can start with: a missing syncword or
/// layer, a reserved rate index, a frame length that does not reach past the header it
/// is stored in, or a frame too short for its protection header.
pub fn header(bytes: &[u8]) -> Option<Header> {
    let b = bytes.get(..7)?;
    // Twelve bits of syncword, then the one-bit version flag and the two-bit
    // layer, which is zero for AAC: masking the version out is what lets both
    // MPEG-2 and MPEG-4 ADTS through while still rejecting a stray 0xFFF of some
    // other format's bytes.
    if b[0] != 0xFF || b[1] & 0xF6 != 0xF0 {
        return None;
    }
    let object_type = ((b[2] >> 6) & 3) + 1;
    let frequency = (b[2] >> 2) & 0xF;
    let channels = (u16::from(b[2] & 1) << 2) | u16::from(b[3] >> 6);
    let frame_bytes =
        (usize::from(b[3] & 3) << 11) | (usize::from(b[4]) << 3) | (usize::from(b[5]) >> 5);
    let raw_blocks = (b[6] & 3) + 1;
    let header_bytes = if b[1] & 1 == 1 { 7 } else { 7 + usize::from(raw_blocks) * 2 };
    if frame_bytes < header_bytes {
        return None;
    }
    let &sample_rate = FREQUENCIES.get(usize::from(frequency))?;
    // The AudioSpecificConfig spells the coding in five bits rather than two, so
    // the low-bit form has to be written out for the common objects and the
    // frame's own rate index and layout follow it in the same widths.
    let asc = [
        (object_type << 3) | (frequency >> 1),
        ((frequency & 1) << 7) | ((channels as u8) << 3),
    ];
    Some(Header {
        sample_rate,
        channels: if channels == 7 {8} else {channels},
        raw_blocks,
        header_bytes,
        frame_bytes,
        asc,
    })
}

fn packet_configuration(header: Header, packet: &[u8]) -> Result<Vec<u8>> {
    if header.channels!=0 {return Ok(header.asc.to_vec());}
    let mut bits=BitReader::new(packet);
    loop {
        match bits.read(3)? {
            4 => skip_data_stream(&mut bits)?,
            5 => break,
            6 => skip_fill(&mut bits)?,
            _ => return Err(invalid("ADTS explicit layout requires PCE before audio in the first packet")),
        }
    }
    let program=ProgramConfig::read(&mut bits,0)?;
    if program.sample_rate!=header.sample_rate || program.object_type!=(header.asc[0]>>3) {return Err(invalid("ADTS PCE disagrees with frame coding or rate"));}
    program.audio_specific_config()
}

/// Return each raw block's byte range within the body following seven fixed
/// header bytes. Verify all multiplexed checks before exposing any block.
fn raw_ranges(fixed: &[u8; 7], body: &[u8], configuration: &[u8]) -> Result<Vec<std::ops::Range<usize>>> {
    let frame = header(fixed).ok_or_else(|| invalid("invalid ADTS header"))?;
    if body.len() != frame.frame_bytes - 7 { return Err(invalid("truncated ADTS frame")); }
    let start = frame.header_bytes - 7;
    if frame.raw_blocks == 1 {
        if start == 2 {
            adts_crc::verify(fixed, u16::from_be_bytes([body[0], body[1]]), &body[start..], configuration).map_err(|e|invalid(&e.0))?;
        }
        return Ok(vec![start..body.len()]);
    }
    let mut ranges = Vec::new();
    if start == 0 {
        let mut at = 0;
        for _ in 0..frame.raw_blocks {
            if at == body.len() { return Err(invalid("ADTS raw block count disagrees with frame length")); }
            let bytes = adts_crc::raw_block_bytes(&body[at..], configuration).map_err(|e|invalid(&e.0))?;
            ranges.push(at..at + bytes); at += bytes;
        }
        if at != body.len() { return Err(invalid("ADTS raw block count disagrees with frame length")); }
    } else {
        let table = start - 2;
        let stored = u16::from_be_bytes([body[table], body[table + 1]]);
        if adts_crc::header_checksum(fixed, &body[..table]) != stored { return Err(invalid("ADTS header CRC mismatch")); }
        let mut boundaries = vec![start];
        for offset in body[..table].chunks_exact(2) {
            let next = start + usize::from(u16::from_be_bytes([offset[0], offset[1]]));
            if next <= *boundaries.last().unwrap() || next >= body.len() { return Err(invalid("invalid ADTS raw block position")); }
            boundaries.push(next);
        }
        boundaries.push(body.len());
        for pair in boundaries.windows(2) {
            let (at, end) = (pair[0], pair[1]);
            if end - at < 3 { return Err(invalid("invalid ADTS raw block position")); }
            let payload = &body[at..end - 2];
            if adts_crc::raw_block_bytes(payload, configuration).map_err(|e|invalid(&e.0))? != payload.len() { return Err(invalid("ADTS raw block position disagrees with syntax")); }
            let stored = u16::from_be_bytes([body[end - 2], body[end - 1]]);
            if adts_crc::raw_block_checksum(payload, configuration).map_err(|e|invalid(&e.0))? != stored { return Err(invalid("ADTS raw block CRC mismatch")); }
            ranges.push(at..end - 2);
        }
    }
    Ok(ranges)
}

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    /// Header bytes to strip; zero for a separately indexed multiplexed raw block.
    pub header_bytes: usize,
    /// The coding and geometry this frame's header states, kept so a test can
    /// name what was read rather than a copy of the numbers the file says, and
    /// so the run can be checked frame against frame.
    pub asc: [u8; 2],
    pub pts: u64,
}

/// A `.aac` file: the geometry its frames state, the setup block synthesized
/// from them, and the frames that play.
#[derive(Clone, Debug)]
pub struct Aac {
    pub sample_rate: u32,
    pub channels: u16,
    /// Samples one frame holds, read out of the setup block rather than assumed.
    pub samples_per_frame: u32,
    pub frames: Vec<Frame>,
    pub configuration: Vec<u8>,
    data: Vec<u8>,
    transport_span: std::ops::Range<usize>,
}

impl Aac {
    /// Read a whole file: the run of frames, from wherever the first header
    /// survives to wherever the last one ends.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        // As in the other elementary readers, the first header may sit behind
        // whatever chopped file got in front of it, and after it the run must be
        // contiguous: each frame starts where the last one's stated length ended.
        let mut found: Vec<Header> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut pos = 0usize;
        let mut raw_count = 0usize;
        while pos + 7 <= bytes.len() && header(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        while let Some(at) = header(bytes.get(pos..).unwrap_or_default()) {
            if pos + at.frame_bytes > bytes.len() {
                // A frame reaching past what the file holds is a truncated tail.
                break;
            }
            raw_count += usize::from(at.raw_blocks);
            if raw_count > limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            found.push(at);
            starts.push(pos);
            pos += at.frame_bytes;
        }
        if found.is_empty() {
            return Err(invalid("no ADTS frames in the file"));
        }
        // A run that stops well short of the end of the file was never the file:
        // a stream of frames fills it, and anything else is another container
        // whose bytes this reader happened to make a syncword out of.
        if (pos - first_start) * 10 < (bytes.len() - first_start) * 9 {
            return Err(invalid(&format!(
                "the frames run to byte {pos} of a file {} bytes long",
                bytes.len()
            )));
        }
        let first = found[0];
        // The setup block is one record for the stream, so the coding the first
        // frame states is checked against the repository's own AAC config
        // parser, which selects the supported Main/LC/SSR object and frame geometry.
        let configuration=packet_configuration(first,&bytes[starts[0]+first.header_bytes..starts[0]+first.frame_bytes])?;
        let config = AacConfig::parse(&configuration)?;
        if config.sample_rate != first.sample_rate || first.channels!=0 && u16::from(config.channels) != first.channels {
            return Err(invalid(
                "the frame header and the AudioSpecificConfig disagree",
            ));
        }
        let mut frames = Vec::with_capacity(found.len());
        let mut pts = 0u64;
        for (at, start) in found.iter().zip(&starts) {
            if at.sample_rate != first.sample_rate
                || at.channels != first.channels
                || (at.header_bytes == 7) != (first.header_bytes == 7)
                || at.asc != first.asc
            {
                return Err(invalid(&format!(
                    "stream changes geometry at frame {}: {} Hz {} ch after {} Hz {} ch",
                    frames.len(),
                    at.sample_rate,
                    at.channels,
                    first.sample_rate,
                    first.channels
                )));
            }
            let fixed: &[u8; 7] = bytes[*start..*start + 7].try_into().unwrap();
            let body = &bytes[*start + 7..*start + at.frame_bytes];
            for range in raw_ranges(fixed, body, &configuration)? {
                if at.raw_blocks == 1 {
                    frames.push(Frame { start: *start, size: at.frame_bytes,
                        header_bytes: at.header_bytes, asc: at.asc, pts });
                } else {
                    frames.push(Frame { start: *start + 7 + range.start, size: range.len(),
                        header_bytes: 0, asc: at.asc, pts });
                }
                pts += u64::from(config.frame_samples);
            }
        }
        Ok(Self {
            sample_rate: first.sample_rate,
            channels: u16::from(config.channels),
            samples_per_frame: u32::from(config.frame_samples),
            frames,
            configuration,
            data: bytes.to_vec(),
            transport_span: first_start..pos,
        })
    }

    /// Every source byte belongs to a complete transport frame, including CRCs.
    /// Recovery parsing may otherwise leave unrepresented leading/trailing data.
    pub fn has_complete_transport(&self) -> bool {
        self.transport_span.start == 0 && self.transport_span.end == self.data.len()
    }

    /// Frames the stream hands over as packets.
    pub fn packets(&self) -> usize {
        self.frames.len()
    }

    /// One frame's audio: its bytes from past the header to its end, which is
    /// the raw block the decoder reads.
    pub fn packet(&self, index: usize) -> &[u8] {
        let at = self.frames[index.min(self.frames.len() - 1)];
        &self.data[at.start + at.header_bytes..at.start + at.size]
    }

    /// Samples the stream runs to, which is its length for a file that states no
    /// duration of its own.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(self.samples_per_frame))
            .unwrap_or(0)
    }

    /// The `esds` payload the AAC decoder asks its setup block for: the frame's
    /// two-byte AudioSpecificConfig inside the descriptors the MP4 sample entry
    /// would have carried.
    pub fn extra_data(&self) -> Vec<u8> {
        esds_for(&self.configuration).expect("validated ADTS initialization is representable")
    }
}

/// Wrap an AudioSpecificConfig in the `esds` descriptors the MP4 sample entry
/// would have carried around it. Two containers hand that config over and
/// neither gives the wrapper: a bare `.aac` file states it in every frame
/// header, and Matroska keeps it in `CodecPrivate` under the tag `A_AAC`.
///
/// The lengths below are the byte counts of the records that follow them, and
/// the bitrates are left at zero, which is what a variable-rate stream states
/// and what the decoder ignores.
pub fn esds_for(asc: &[u8]) -> Option<Vec<u8>> {
    esds_descriptor(asc, false)
}

/// MP4 ES descriptor with terminal predefined SL configuration.
pub(crate) fn esds_for_mp4(asc: &[u8]) -> Option<Vec<u8>> {
    esds_descriptor(asc, true)
}

fn esds_descriptor(asc: &[u8], sl_config: bool) -> Option<Vec<u8>> {
    if !(2..=4096).contains(&asc.len()) {return None;}
    fn descriptor(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut groups=vec![(payload.len() & 127) as u8];
        let mut length=payload.len() >> 7;
        while length!=0 {groups.push((length & 127) as u8 | 128);length>>=7;}
        groups.reverse();
        let mut bytes=vec![tag];bytes.extend(groups);bytes.extend_from_slice(payload);bytes
    }
    let mut decoder=vec![0x40,0x15,0,0,0,0,0,0,0,0,0,0,0];
    decoder.extend(descriptor(5,asc));
    let mut stream=vec![0,1,0];stream.extend(descriptor(4,&decoder));
    if sl_config {stream.extend(descriptor(6, &[2]));}
    let mut output=vec![0;4];output.extend(descriptor(3,&stream));Some(output)
}

fn check_packet_limit(header: Header, max: usize) -> Result<()> {
    if header.frame_bytes - header.header_bytes > max {
        return Err(invalid("ADTS packet exceeds budget"));
    }
    Ok(())
}

/// Sequential ADTS reader. Retains blocks from at most one transport frame
/// (ADTS length is 13 bits),
/// with no file-size or packet-count allocation. Input starts at an ADTS header;
/// unlike the recovery-oriented slice parser, truncated tails are errors.
pub struct StreamReader<R> {
    source: R,
    configuration: Header,
    first: Option<(Header, [u8; 7])>,
    pending: std::collections::VecDeque<Vec<u8>>,
    asc: Vec<u8>,
    finished: bool,
    max_packet_bytes: usize,
}
impl<R: std::io::Read> StreamReader<R> {
    pub fn open(source: R) -> Result<Self> {
        Self::open_with_packet_limit(source, usize::MAX)
    }

    /// Bound retained transport payload before allocating it, including PCE
    /// bootstrap. A multiplexed frame's payload shares this one frame budget.
    /// ADTS headers and an optional two-byte CRC are framing, not payload.
    pub fn open_with_packet_limit(mut source: R, max_packet_bytes: usize) -> Result<Self> {
        let mut bytes = [0; 7];
        source.read_exact(&mut bytes)?;
        let mut configuration = header(&bytes).ok_or_else(|| invalid("invalid ADTS header"))?;
        check_packet_limit(configuration, max_packet_bytes)?;
        let mut pending=std::collections::VecDeque::new();
        let asc=if configuration.channels==0 {
            let mut packet=vec![0;configuration.frame_bytes-7];source.read_exact(&mut packet)?;
            let start = configuration.header_bytes - 7;
            let asc=packet_configuration(configuration,&packet[start..])?;
            for range in raw_ranges(&bytes, &packet, &asc)? { pending.push_back(packet[range].to_vec()); }
            asc
        } else {configuration.asc.to_vec()};
        let config = AacConfig::parse(&asc)?;
        if configuration.channels==0 {configuration.channels=u16::from(config.channels);}
        if config.sample_rate != configuration.sample_rate
            || u16::from(config.channels) != configuration.channels
        {
            return Err(invalid("ADTS configuration disagrees with header"));
        }
        Ok(Self {
            source,
            configuration,
            first: if !pending.is_empty() {None} else {Some((configuration, bytes))},
            pending, asc,
            finished: false,
            max_packet_bytes,
        })
    }

    pub fn configuration(&self) -> Header {
        self.configuration
    }

    pub fn audio_specific_config(&self) -> &[u8] {&self.asc}

    /// Return the raw AAC block, excluding ADTS header/CRC. An error terminates
    /// this reader; callers must not publish partially decoded output as success.
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>> {
        if self.finished {
            return Ok(None);
        }
        if let Some(packet)=self.pending.pop_front() {return Ok(Some(packet));}
        self.finished = true;
        let (next, fixed) = if let Some(header) = self.first.take() {
            header
        } else {
            let mut bytes = [0; 7];
            match self.source.read_exact(&mut bytes[..1]) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(error) => return Err(error.into()),
            }
            self.source.read_exact(&mut bytes[1..])?;
            (header(&bytes).ok_or_else(|| invalid("invalid ADTS frame boundary"))?, bytes)
        };
        if next.asc != self.configuration.asc
            || (next.header_bytes == 7) != (self.configuration.header_bytes == 7)
        {
            return Err(invalid("ADTS configuration changes between frames"));
        }
        check_packet_limit(next, self.max_packet_bytes)?;
        let mut remaining = vec![0; next.frame_bytes - 7];
        self.source.read_exact(&mut remaining)?;
        for range in raw_ranges(&fixed, &remaining, &self.asc)? { self.pending.push_back(remaining[range].to_vec()); }
        self.finished = false;
        Ok(self.pending.pop_front())
    }
}

/// A sequence of independently framed ADTS inputs with identical AAC setup.
/// EOF advances to the next reader; a malformed/truncated segment poisons the
/// sequence instead of taking any bytes from another input to complete a frame.
pub struct SequenceReader<R> {
    remaining: std::vec::IntoIter<StreamReader<R>>,
    current: Option<StreamReader<R>>,
    configuration: Header,
    asc: Vec<u8>,
    failed: bool,
}
impl<R:std::io::Read> SequenceReader<R> {
    pub fn new(readers: Vec<StreamReader<R>>) -> Result<Self> {
        if !(2..=256).contains(&readers.len()) {return Err(invalid("concat requires 2..=256 inputs"));}
        let configuration=readers[0].configuration();
        let asc=readers[0].audio_specific_config().to_vec();
        for reader in &readers {
            let other=reader.configuration();
            if reader.audio_specific_config()!=asc || (configuration.asc,configuration.channels,configuration.sample_rate)!=(other.asc,other.channels,other.sample_rate) {
                return Err(invalid("ADTS concat requires identical AAC configurations"));
            }
        }
        let mut remaining=readers.into_iter();let current=remaining.next();
        Ok(Self {remaining,current,configuration,asc,failed:false})
    }
    pub fn configuration(&self)->Header {self.configuration}
    pub fn audio_specific_config(&self)->&[u8] {&self.asc}
    pub fn next_packet(&mut self)->Result<Option<Vec<u8>>> {
        if self.failed {return Err(invalid("ADTS sequence failed"));}
        loop {
            let Some(reader)=self.current.as_mut() else {return Ok(None);};
            match reader.next_packet() {
                Ok(Some(packet))=>return Ok(Some(packet)),
                Ok(None)=>self.current=self.remaining.next(),
                Err(error)=>{self.failed=true;return Err(error);},
            }
        }
    }
}
