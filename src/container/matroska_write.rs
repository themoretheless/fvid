//! Owned AVC/HEVC/AAC Matroska muxing. No external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
fn size(value: u64) -> Result<Vec<u8>> {
    for width in 1..=8 {
        if value < (1u64 << (7 * width)) - 1 {
            let encoded = (value | (1u64 << (7 * width))).to_be_bytes();
            return Ok(encoded[8 - width..].to_vec());
        }
    }
    Err(invalid("Matroska element exceeds size range"))
}
fn head(output: &mut impl Write, id: u32, length: u64) -> Result<()> {
    let bytes = id.to_be_bytes();
    let start = bytes
        .iter()
        .position(|&b| b != 0)
        .ok_or_else(|| invalid("zero EBML ID"))?;
    output.write_all(&bytes[start..])?;
    output.write_all(&size(length)?)?;
    Ok(())
}
fn element(id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    head(&mut out, id, data.len() as u64)?;
    out.extend_from_slice(data);
    Ok(out)
}
fn uint(id: u32, value: u64) -> Result<Vec<u8>> {
    let b = value.to_be_bytes();
    element(id, &b[b.iter().position(|&v| v != 0).unwrap_or(7)..])
}

/// Packet storage is unchanged: length-prefixed AVC/HEVC NAL units or raw AAC.
pub enum Encoding<'a> {
    Avc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Hevc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Aac {
        configuration: &'a [u8],
        sample_rate: u32,
        channels: u16,
    },
}

/// Description of an encoded track on the caller's presentation timeline.
/// Container edit lists, rotation, colour/HDR metadata and gapless trimming
/// must be mapped by a higher-level remuxer before using this packet interface.
pub struct TrackSpec<'a> {
    pub encoding: Encoding<'a>,
    pub name: &'a str,
    pub language: &'a str,
}

fn track_entry(spec: &TrackSpec<'_>, number: u64) -> Result<Vec<u8>> {
    use crate::codec::config::{AacConfig, AvcConfig, HevcConfig};
    let (id, config, kind, geometry) = match spec.encoding {
        Encoding::Avc {
            configuration,
            width,
            height,
        } => {
            AvcConfig::parse(configuration)?;
            ("V_MPEG4/ISO/AVC", configuration, 1, video(width, height)?)
        }
        Encoding::Hevc {
            configuration,
            width,
            height,
        } => {
            HevcConfig::parse(configuration)?;
            ("V_MPEGH/ISO/HEVC", configuration, 1, video(width, height)?)
        }
        Encoding::Aac {
            configuration,
            sample_rate,
            channels,
        } => {
            let config = AacConfig::parse(configuration)?;
            if sample_rate == 0
                || channels == 0
                || config.sample_rate != sample_rate
                || u16::from(config.channels) != channels
            {
                return Err(invalid("AAC Matroska geometry differs from configuration"));
            }
            (
                "A_AAC",
                configuration,
                2,
                element(
                    0xe1,
                    &[
                        element(0xb5, &f64::from(sample_rate).to_be_bytes())?,
                        uint(0x9f, u64::from(channels))?,
                    ]
                    .concat(),
                )?,
            )
        }
    };
    let mut data = [
        uint(0xd7, number)?,
        uint(0x73c5, number)?,
        uint(0x83, kind)?,
        uint(0x9c, 0)?,
        element(0x86, id.as_bytes())?,
        element(0x63a2, config)?,
        geometry,
    ]
    .concat();
    if !spec.name.is_empty() {
        data.extend(element(0x536e, spec.name.as_bytes())?);
    }
    // Write an explicit undetermined language instead of Matroska's default eng.
    data.extend(element(
        0x22b59c,
        if spec.language.is_empty() {
            b"und"
        } else {
            spec.language.as_bytes()
        },
    )?);
    element(0xae, &data)
}
fn video(width: u32, height: u32) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(invalid("empty Matroska video dimensions"));
    }
    element(
        0xe0,
        &[
            uint(0xb0, u64::from(width))?,
            uint(0xba, u64::from(height))?,
        ]
        .concat(),
    )
}

/// Streaming packet writer. Feed each track in decode order, interleaving tracks
/// at the call site. PTS can move backwards for B-frames; each packet has its own
/// nanosecond-clock cluster, avoiding signed 16-bit block timestamp overflow.
/// No payload copies or accumulated packet index. Discard output on any error.
pub struct PacketWriter<'a, W> {
    output: &'a mut W,
    segment_size: u64,
    duration_offset: u64,
    written: Vec<bool>,
    end_ns: u64,
    event: ProgressEvent,
    failed: bool,
}
impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    pub fn new(output: &'a mut W, tracks: &[TrackSpec<'_>]) -> Result<Self> {
        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        let mut entries = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            entries.extend(track_entry(track, index as u64 + 1)?);
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
        output.write_all(&element(0x1654ae6b, &entries)?)?;
        Ok(Self {
            output,
            segment_size,
            duration_offset,
            written: vec![false; tracks.len()],
            end_ns: 0,
            event: ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
            failed: false,
        })
    }
    pub fn event(&self) -> ProgressEvent {
        self.event
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
        if self.failed {
            return Err(invalid("Matroska writer failed"));
        }
        let result = self.packet(track, pts_ns, duration_ns, sync, payload);
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
    ) -> Result<()> {
        if track >= self.written.len() || duration_ns == 0 || payload.is_empty() {
            return Err(invalid("invalid Matroska packet"));
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
        let block = 4 + payload.len() as u64;
        let group =
            1 + size(block)?.len() as u64 + block + duration.len() as u64 + reference.len() as u64;
        let cluster = timestamp.len() as u64 + 1 + size(group)?.len() as u64 + group;
        head(self.output, 0x1f43b675, cluster)?;
        self.output.write_all(&timestamp)?;
        head(self.output, 0xa0, group)?;
        head(self.output, 0xa1, block)?;
        self.output
            .write_all(&[0x80 | (track as u8 + 1), 0, 0, 0])?;
        self.output.write_all(payload)?;
        self.output.write_all(&duration)?;
        self.output.write_all(&reference)?;
        self.written[track] = true;
        self.end_ns = self.end_ns.max(end);
        self.event.packets = packets;
        self.event.payload_bytes = bytes;
        Ok(())
    }
    /// Patch finite segment size and presentation duration. `done` remains false:
    /// the caller owns flushing, syncing and atomic publication.
    pub fn finish(self) -> Result<ProgressEvent> {
        if self.failed || self.written.iter().any(|v| !v) {
            return Err(invalid("incomplete Matroska tracks"));
        }
        let end = self.output.stream_position()?;
        let length = end - self.segment_size - 8;
        if length >= (1u64 << 56) - 1 {
            return Err(invalid("Matroska segment exceeds size range"));
        }
        self.output.seek(SeekFrom::Start(self.segment_size))?;
        self.output
            .write_all(&(length | (1u64 << 56)).to_be_bytes())?;
        self.output.seek(SeekFrom::Start(self.duration_offset))?;
        self.output.write_all(&(self.end_ns as f64).to_be_bytes())?;
        self.output.seek(SeekFrom::Start(end))?;
        Ok(self.event)
    }
}

/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    mut input: super::adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_aac_packets(
        input.configuration(),
        || input.next_packet(),
        output,
        cancel,
        progress,
    )
}

/// Append compatible ADTS segments to one Matroska track without decoding.
pub fn concat_adts<R: Read, W: Write + Seek>(
    readers: Vec<super::adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let mut sequence = super::adts::SequenceReader::new(readers)?;
    write_aac_packets(
        sequence.configuration(),
        || sequence.next_packet(),
        output,
        cancel,
        progress,
    )
}

fn write_aac_packets<W: Write + Seek>(
    config: super::adts::Header,
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let samples = u64::from(crate::codec::config::AacConfig::parse(&config.asc)?.frame_samples);
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: &config.asc,
            sample_rate: config.sample_rate,
            channels: config.channels.into(),
        },
        name: "",
        language: "",
    };
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    let time = |packet: u64| -> Result<u64> {
        u64::try_from(
            u128::from(packet) * u128::from(samples) * 1_000_000_000
                / u128::from(config.sample_rate),
        )
        .map_err(|_| invalid("Matroska timestamp overflow"))
    };
    check()?;
    let mut writer = PacketWriter::new(output, &[spec])?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    loop {
        check()?;
        let Some(packet) = next_packet()? else {
            break;
        };
        let index = writer.event().packets;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let start = time(index)?;
        writer.write_packet(0, start, time(next)? - start, true, &packet)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    let event = writer.finish()?;
    check()?;
    Ok(event)
}
