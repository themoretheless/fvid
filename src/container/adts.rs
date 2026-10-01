//! Owned ADTS framing and AAC configuration, independent of playback.
use crate::codec::config::AacConfig;
use crate::{Result, invalid};

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

/// The tag [`crate::codec::make_audio_decoder`] answers for AAC. It is the MP4
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
    /// Bytes of header, before the raw block: seven with no CRC, nine with one.
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
/// is stored in, or a frame holding several raw blocks.
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
    let header_bytes = if b[1] & 1 == 1 { 7 } else { 9 };
    if frame_bytes < header_bytes {
        return None;
    }
    // Two bits at the end of the fixed header count the raw blocks after the
    // first; a frame that holds two holds two sets of samples, and the running
    // count of 1024 per frame would fall behind them.
    if b[6] & 3 != 0 {
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
        header_bytes,
        frame_bytes,
        asc,
    })
}

fn packet_configuration(header: Header, packet: &[u8]) -> Result<Vec<u8>> {
    if header.channels!=0 {return Ok(header.asc.to_vec());}
    let mut bits=crate::codec::bits::BitReader::new(packet);
    loop {
        match bits.read(3)? {
            4 => crate::codec::aac_pce::skip_data_stream(&mut bits)?,
            5 => break,
            _ => return Err(invalid("ADTS explicit layout requires PCE before audio in the first packet")),
        }
    }
    let program=crate::codec::aac_pce::ProgramConfig::read(&mut bits,0)?;
    if program.sample_rate!=header.sample_rate || program.object_type!=2 || header.asc[0]>>3!=2 {return Err(invalid("ADTS PCE disagrees with frame coding or rate"));}
    program.audio_specific_config()
}

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    /// Bytes of ADTS header on this frame, which the packet leaves off.
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
        while pos + 7 <= bytes.len() && header(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        while let Some(at) = header(bytes.get(pos..).unwrap_or_default()) {
            if pos + at.frame_bytes > bytes.len() {
                // A frame reaching past what the file holds is a truncated tail.
                break;
            }
            if found.len() >= limits.packets {
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
        // parser: it is what names AAC-LC as the only object this build decodes.
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
                || at.header_bytes != first.header_bytes
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
            frames.push(Frame {
                start: *start,
                size: at.frame_bytes,
                header_bytes: at.header_bytes,
                asc: at.asc,
                pts,
            });
            pts += u64::from(config.frame_samples);
        }
        Ok(Self {
            sample_rate: first.sample_rate,
            channels: u16::from(config.channels),
            samples_per_frame: u32::from(config.frame_samples),
            frames,
            configuration,
            data: bytes.to_vec(),
        })
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

/// Sequential ADTS reader. Retains at most one frame (ADTS length is 13 bits),
/// with no file-size or packet-count allocation. Input starts at an ADTS header;
/// unlike the recovery-oriented slice parser, truncated tails are errors.
pub struct StreamReader<R> {
    source: R,
    configuration: Header,
    first: Option<Header>,
    pending: Option<Vec<u8>>,
    asc: Vec<u8>,
    finished: bool,
}
impl<R: std::io::Read> StreamReader<R> {
    pub fn open(mut source: R) -> Result<Self> {
        let mut bytes = [0; 7];
        source.read_exact(&mut bytes)?;
        let mut configuration = header(&bytes).ok_or_else(|| invalid("invalid ADTS header"))?;
        let mut pending=None;
        let asc=if configuration.channels==0 {
            let mut packet=vec![0;configuration.frame_bytes-7];source.read_exact(&mut packet)?;
            if configuration.header_bytes==9 {packet.drain(..2);}
            let asc=packet_configuration(configuration,&packet)?;
            pending=Some(packet);asc
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
            first: if pending.is_some() {None} else {Some(configuration)},
            pending, asc,
            finished: false,
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
        if let Some(packet)=self.pending.take() {return Ok(Some(packet));}
        self.finished = true;
        let next = if let Some(header) = self.first.take() {
            header
        } else {
            let mut bytes = [0; 7];
            match self.source.read_exact(&mut bytes[..1]) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(error) => return Err(error.into()),
            }
            self.source.read_exact(&mut bytes[1..])?;
            header(&bytes).ok_or_else(|| invalid("invalid ADTS frame boundary"))?
        };
        if next.asc != self.configuration.asc
            || next.header_bytes != self.configuration.header_bytes
        {
            return Err(invalid("ADTS configuration changes between frames"));
        }
        let mut remaining = vec![0; next.frame_bytes - 7];
        self.source.read_exact(&mut remaining)?;
        if next.header_bytes == 9 {
            remaining.drain(..2);
        }
        self.finished = false;
        Ok(Some(remaining))
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

#[cfg(test)]
mod mp4_descriptor_tests {
    #[test]
    fn sl_configuration_is_inside_multibyte_es_descriptor() {
        for width in [2, 107, 108, 127, 128, 255, 4096] {
            let mut asc = vec![0; width];
            asc[..2].copy_from_slice(&[0x11, 0x90]);
            let descriptor = super::esds_for_mp4(&asc).unwrap();
            assert_eq!(crate::codec::config::aac_specific_config(&descriptor).unwrap(), asc);
            let mut length = 0usize;
            let mut cursor = 5;
            loop {
                let byte = descriptor[cursor];
                cursor += 1;
                length = (length << 7) | usize::from(byte & 127);
                if byte & 128 == 0 {break;}
            }
            assert_eq!(cursor + length, descriptor.len());
            assert_eq!(&descriptor[descriptor.len()-3..], &[6, 1, 2]);
        }
    }
}
