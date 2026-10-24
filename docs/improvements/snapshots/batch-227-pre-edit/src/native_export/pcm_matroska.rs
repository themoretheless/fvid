//! Bounded float PCM packetization for the owned audio export pipeline.
use crate::{
    Result,
    container::matroska_write::{Encoding, PacketWriter, TrackSpec},
    invalid,
};
use std::io::{self, Seek, Write};

pub(crate) enum Output<'a, W> {
    Raw(&'a mut W),
    Matroska(Sink<'a, W>),
}
impl<'a, W: Write + Seek> Output<'a, W> {
    pub(crate) fn new_with_options(
        output: &'a mut W,
        matroska: bool,
        rate: u32,
        channels: u16,
        options: Option<&fvid_media::CopyOptions>,
    ) -> Result<Self> {
        if matroska {
            Ok(Self::Matroska(Sink::new(output, rate, channels, options)?))
        } else {
            Ok(Self::Raw(output))
        }
    }
    pub(crate) fn finish(self) -> Result<Option<u64>> {
        match self {
            Self::Raw(_) => Ok(None),
            Self::Matroska(sink) => sink.finish().map(Some),
        }
    }
}
impl<W: Write + Seek> Write for Output<'_, W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            Self::Raw(output) => output.write(data),
            Self::Matroska(sink) => sink.write(data),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Raw(output) => output.flush(),
            Self::Matroska(sink) => sink.flush(),
        }
    }
}

type MatroskaPcmMetadata = (
    String,
    String,
    crate::container::matroska_write::FileMetadata,
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
);

pub(crate) fn validate_matroska_pcm_metadata(
    options: Option<&fvid_media::CopyOptions>,
) -> Result<()> {
    parse_matroska_pcm_metadata(options).map(|_| ())
}

fn parse_matroska_pcm_metadata(
    options: Option<&fvid_media::CopyOptions>,
) -> Result<MatroskaPcmMetadata> {
    let mut name = String::new();
    let mut language = String::from("und");
    let mut metadata = crate::container::matroska_write::FileMetadata::default();
    let mut text = std::collections::BTreeMap::new();
    let mut scoped = std::collections::BTreeMap::<
        usize,
        std::collections::BTreeMap<String, String>,
    >::new();
    if let Some(o) = options {
        if o.metadata_set.len() + o.metadata_delete.len() > 64 {
            return Err(invalid("at most 64 container metadata mutations"));
        }
        if o.stream_metadata_set.len() + o.stream_metadata_delete.len() > 64 {
            return Err(invalid("at most 64 stream metadata mutations"));
        }
        for key in &o.metadata_delete {
            if key.is_empty() || key.len() > 128 || key.contains('\0') {
                return Err(invalid("invalid Matroska container metadata key"));
            }
            metadata.tags.set(key, "");
            text.remove(key);
        }
        for (key, value) in &o.metadata_set {
            if key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
            {
                return Err(invalid("invalid Matroska container metadata"));
            }
            if !metadata.tags.set(key, value) {
                text.insert(key.clone(), value.clone());
            }
        }
        for (index, key) in &o.stream_metadata_delete {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || key.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
            {
                return Err(invalid("invalid Matroska stream metadata deletion"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clear();
            } else if key.eq_ignore_ascii_case("language") {
                language = "und".into();
            } else if let Some(entries) = scoped.get_mut(&0) {
                entries.remove(key);
            }
        }
        for (index, key, value) in &o.stream_metadata_set {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
                || (key.eq_ignore_ascii_case("language") && value.len() > 128)
            {
                return Err(invalid("invalid Matroska stream metadata"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clone_from(value);
            } else if key.eq_ignore_ascii_case("language") {
                language = if value.is_empty() {
                    "und".into()
                } else {
                    value.clone()
                };
            } else {
                scoped
                    .entry(0)
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
        }
    }
    Ok((name, language, metadata, text, scoped))
}
pub(crate) struct Sink<'a, W> {
    writer: PacketWriter<'a, W>,
    rate: u32,
    frame_bytes: usize,
    frames: u64,
    bytes: Vec<u8>,
    block_bytes: usize,
}
impl<'a, W: Write + Seek> Sink<'a, W> {
    fn new(
        output: &'a mut W,
        rate: u32,
        channels: u16,
        options: Option<&fvid_media::CopyOptions>,
    ) -> Result<Self> {
        let (name, language, metadata, text, scoped) = parse_matroska_pcm_metadata(options)?;
        let spec = [TrackSpec {
            encoding: Encoding::PcmFloat32 {
                sample_rate: rate,
                channels,
            },
            name: &name,
            language: &language,
        }];
        let writer =
            PacketWriter::new_with_text_metadata(output, &spec, &[], &metadata, &text, &scoped)?;
        let frame_bytes = usize::from(channels) * 4;
        let block_bytes = frame_bytes * 1024;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(block_bytes)
            .map_err(|_| invalid("cannot allocate PCM packet buffer"))?;
        Ok(Self {
            writer,
            rate,
            frame_bytes,
            frames: 0,
            bytes,
            block_bytes,
        })
    }
    fn emit(&mut self) -> Result<()> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        if !self.bytes.len().is_multiple_of(self.frame_bytes) {
            return Err(invalid("incomplete Matroska PCM sample frame"));
        }
        let next = self
            .frames
            .checked_add((self.bytes.len() / self.frame_bytes) as u64)
            .ok_or_else(|| invalid("PCM sample clock overflow"))?;
        let time = |frame: u64| {
            u64::try_from(u128::from(frame) * 1_000_000_000 / u128::from(self.rate))
                .map_err(|_| invalid("PCM timestamp overflow"))
        };
        let begin = time(self.frames)?;
        let end = time(next)?;
        self.writer
            .write_packet(0, begin, end - begin, true, &self.bytes)?;
        self.frames = next;
        self.bytes.clear();
        Ok(())
    }
    fn finish(mut self) -> Result<u64> {
        self.emit()?;
        self.writer.finish()?;
        Ok(self.frames)
    }
}
impl<W: Write + Seek> Write for Sink<'_, W> {
    fn write(&mut self, mut data: &[u8]) -> io::Result<usize> {
        let count = data.len();
        while !data.is_empty() {
            let take = data.len().min(self.block_bytes - self.bytes.len());
            self.bytes.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.bytes.len() == self.block_bytes {
                self.emit().map_err(io::Error::other)?;
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.emit().map_err(io::Error::other)?;
        self.writer.flush().map_err(io::Error::other)
    }
}
