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
    #[allow(dead_code)]
    pub(crate) fn new_with_options(
        output: &'a mut W,
        matroska: bool,
        rate: u32,
        channels: u16,
        options: Option<&fvid_media::CopyOptions>,
    ) -> Result<Self> {
        Self::new_with_source_metadata(output, matroska, rate, channels, None, options)
    }
    pub(crate) fn new_with_source_metadata(
        output: &'a mut W,
        matroska: bool,
        rate: u32,
        channels: u16,
        base: Option<MatroskaPcmMetadata>,
        options: Option<&fvid_media::CopyOptions>,
    ) -> Result<Self> {
        if matroska {
            Ok(Self::Matroska(Sink::new_with_source_metadata(
                output, rate, channels, base, options,
            )?))
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

pub(crate) type MatroskaPcmMetadata = (
    String,
    String,
    crate::container::matroska_write::FileMetadata,
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
);

pub(crate) fn default_matroska_pcm_metadata() -> MatroskaPcmMetadata {
    (
        String::new(),
        String::from("und"),
        crate::container::matroska_write::FileMetadata::default(),
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
    )
}

#[allow(dead_code)]
pub(crate) fn extract_source_pcm_metadata(
    source: &std::path::Path,
    selected: Option<usize>,
    allow_wave: bool,
    max_packet_bytes: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<MatroskaPcmMetadata> {
    extract_source_pcm_metadata_with_packet_limit(
        source,
        selected,
        allow_wave,
        max_packet_bytes,
        None,
        cancel,
    )
}

pub(crate) fn extract_source_pcm_metadata_with_packet_limit(
    source: &std::path::Path,
    selected: Option<usize>,
    allow_wave: bool,
    max_packet_bytes: Option<usize>,
    max_packets: Option<u64>,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<MatroskaPcmMetadata> {
    extract_source_pcm_metadata_with_interval_and_packet_limit(
        source,
        selected,
        allow_wave,
        None,
        max_packet_bytes,
        max_packets,
        cancel,
    )
}

pub(crate) fn extract_source_pcm_metadata_with_interval_and_packet_limit(
    source: &std::path::Path,
    selected: Option<usize>,
    allow_wave: bool,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    max_packet_bytes: Option<usize>,
    max_packets: Option<u64>,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<MatroskaPcmMetadata> {
    extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
        source,
        selected,
        allow_wave,
        interval,
        max_packet_bytes,
        max_packets,
        None,
        cancel,
    )
}

pub(crate) fn extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
    source: &std::path::Path,
    selected: Option<usize>,
    allow_wave: bool,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    max_packet_bytes: Option<usize>,
    max_packets: Option<u64>,
    max_controlled_bytes: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<MatroskaPcmMetadata> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0u8; 8];
    if input.read_exact(&mut prefix).is_err() {
        return Ok(default_matroska_pcm_metadata());
    }
    input.seek(SeekFrom::Start(0))?;
    if allow_wave && matches!(&prefix[..4], b"RIFF" | b"RF64" | b"RIFX") {
        let mut base = default_matroska_pcm_metadata();
        if let Ok(info) = crate::media::probe(source) {
            for (key, value) in info.metadata {
                if value.is_empty() || value.contains('\0') || value.len() > 1024 {
                    continue;
                }
                if key.eq_ignore_ascii_case("ILNG") {
                    if value.len() <= 128 {
                        base.1 = value.clone();
                    }
                    base.3.insert("ILNG".into(), value);
                } else if key.eq_ignore_ascii_case("encoder") {
                    base.3.insert("ENCODER".into(), value);
                } else if !base.2.tags.set(&key, &value) && key.len() <= 128 {
                    base.3.insert(key.to_ascii_uppercase(), value);
                }
            }
        }
        return Ok(base);
    }
    if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open_indexed(
            input,
            crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )?;
        let index = if allow_wave {
            let automatic = if selected.is_none() {
                reader.automatic_track_indices(&[*b"soun"])
            } else {
                Vec::new()
            };
            let candidates: Vec<_> = reader
                .tracks()
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    (t.handler == *b"soun" && (selected.is_some() || automatic.contains(&i)))
                        .then_some(i)
                })
                .collect();
            match selected {
                Some(i) if candidates.contains(&i) => i,
                None if candidates.len() == 1 => candidates[0],
                _ => crate::native_media::mp4_audio_index(&reader, selected)?,
            }
        } else {
            crate::native_media::mp4_aac_index(&reader, selected)?
        };
        let track = &reader.tracks()[index];
        if let Some(maximum) = max_packet_bytes {
            let max_samples = crate::native_media::mp4_active_samples(track, interval, max_packets);
            let exceeds = match &track.samples {
                crate::container::mp4::SampleIndex::Expanded(samples) => samples
                    .iter()
                    .take(max_samples)
                    .any(|s| (s.size as usize) > maximum),
                crate::container::mp4::SampleIndex::Uniform(uniform) => {
                    uniform.count > 0
                        && max_samples > 0
                        && (uniform.bytes_per_frame as usize) > maximum
                }
            };
            if exceeds {
                return Err(invalid("MP4 packet exceeds budget"));
            }
        }
        let mut base = default_matroska_pcm_metadata();
        base.0 = track.name.clone();
        if !track.language.is_empty() && track.language.len() <= 128 && !track.language.contains('\0') {
            base.1 = track.language.clone();
        }
        base.2.tags = reader.tags().clone();
        if !reader.encoder().is_empty()
            && !reader.encoder().contains('\0')
            && reader.encoder().len() <= 1024
        {
            base.3.insert("ENCODER".into(), reader.encoder().to_owned());
        }
        return Ok(base);
    }
    if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let mut reader = crate::container::webm::WebmReader::open(
            input,
            crate::container::webm::Limits::default(),
        )?;
        if let Some(maximum) = max_packet_bytes {
            reader.restrict_packet_bytes(maximum);
        }
        reader.scan_all()?;
        let index = if allow_wave {
            let preferred = if selected.is_none() {
                reader.preferred_track_numbers(
                    &reader
                        .tracks
                        .iter()
                        .filter(|t| t.kind == 2)
                        .map(|t| t.number)
                        .collect::<Vec<_>>(),
                )
            } else {
                Vec::new()
            };
            let candidates: Vec<_> = reader
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    (t.kind == 2 && (selected.is_some() || preferred.contains(&t.number)))
                        .then_some(i)
                })
                .collect();
            match selected {
                Some(i) if candidates.contains(&i) => i,
                None if candidates.len() == 1 => candidates[0],
                _ => crate::native_media::matroska_audio_index(&reader, selected)?,
            }
        } else {
            crate::native_media::matroska_aac_index(&reader, selected)?
        };
        let track = &reader.tracks[index];
        if let Some(maximum) = max_packet_bytes {
            let track_number = track.number;
            let active_limit = crate::native_media::matroska_active_track_packets(
                &reader,
                index,
                interval,
                max_packets,
            );
            let mut track_packets = 0usize;
            for (packet_index, packet) in reader.packets.iter().enumerate() {
                if packet.track == track_number {
                    if track_packets >= active_limit {
                        break;
                    }
                    if reader.packet_payload_size(packet_index)? > maximum {
                        return Err(invalid("WebM packet exceeds budget"));
                    }
                    track_packets += 1;
                }
            }
        }
        let mut base = default_matroska_pcm_metadata();
        if !track.name.contains('\0') && track.name.len() <= 1024 {
            base.0 = track.name.clone();
        }
        let lang = reader
            .track_languages
            .get(&track.number)
            .unwrap_or(&track.language);
        if !lang.is_empty() && !lang.contains('\0') && lang.len() <= 128 {
            base.1 = lang.clone();
        }
        base.2.tags = reader.tags.clone();
        for (k, v) in &reader.metadata {
            if !k.is_empty()
                && k.len() <= 128
                && v.len() <= 1024
                && !k.contains('\0')
                && !v.contains('\0')
                && !crate::container::FileTags::supports_key(k)
            {
                base.3.insert(k.clone(), v.clone());
            }
        }
        if !reader.writing_app.is_empty()
            && !reader.writing_app.contains('\0')
            && reader.writing_app.len() <= 1024
            && !base.3.keys().any(|k| k.eq_ignore_ascii_case("encoder"))
        {
            base.3.insert("ENCODER".into(), reader.writing_app.clone());
        }
        let mut track_tags = reader.track_metadata.get(&0).cloned().unwrap_or_default();
        if let Some(uid) = reader.track_uids.get(&track.number)
            && let Some(scoped) = reader.track_metadata.get(uid)
        {
            track_tags.extend(scoped.clone());
        }
        track_tags.retain(|k, v| {
            !k.is_empty()
                && k.len() <= 128
                && v.len() <= 1024
                && !k.contains('\0')
                && !v.contains('\0')
                && !matches!(
                    k.to_ascii_lowercase().as_str(),
                    "title" | "language" | "rotate" | "stereo_mode" | "alpha_mode"
                )
        });
        if !track_tags.is_empty() {
            base.4.insert(0, track_tags);
        }
        return Ok(base);
    }
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    let mut base = default_matroska_pcm_metadata();
    if let Ok(info) = crate::media::probe(source) {
        for (key, value) in info.metadata {
            if value.is_empty() || value.contains('\0') || value.len() > 1024 {
                continue;
            }
            if key.eq_ignore_ascii_case("encoder") {
                base.3.insert("ENCODER".into(), value);
            } else if key.eq_ignore_ascii_case("language") {
                if base.1 == "und" && value.len() <= 128 {
                    base.1 = value;
                }
            } else if !base.2.tags.set(&key, &value) && !key.is_empty() && key.len() <= 128 && !key.contains('\0') {
                base.3.insert(key.to_ascii_uppercase(), value);
            }
        }
        let stream = selected
            .and_then(|idx| info.streams.iter().find(|s| s.index == idx))
            .or_else(|| info.streams.iter().find(|s| s.media_type == "audio"));
        if let Some(stream) = stream {
            let mut track_tags = std::collections::BTreeMap::new();
            for (k, v) in &stream.metadata {
                if v.is_empty() || v.contains('\0') || v.len() > 1024 {
                    continue;
                }
                if k.eq_ignore_ascii_case("title") {
                    base.0.clone_from(v);
                } else if k.eq_ignore_ascii_case("language") {
                    if v.len() <= 128 {
                        base.1.clone_from(v);
                    }
                } else if !k.is_empty()
                    && k.len() <= 128
                    && !k.contains('\0')
                    && !matches!(
                        k.to_ascii_lowercase().as_str(),
                        "rotate" | "stereo_mode" | "alpha_mode"
                    )
                {
                    track_tags.insert(k.to_ascii_uppercase(), v.clone());
                }
            }
            if !track_tags.is_empty() {
                base.4.insert(0, track_tags);
            }
        }
    }
    Ok(base)
}

pub(crate) fn extract_source_wave_info(
    source: &std::path::Path,
    options: &fvid_media::CopyOptions,
) -> Result<Vec<u8>> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let cancel = options.cancel.as_ref();
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0u8; 8];
    if input.read_exact(&mut prefix).is_err() {
        return Ok(Vec::new());
    }
    input.seek(SeekFrom::Start(0))?;
    if matches!(&prefix[..4], b"RIFF" | b"RF64" | b"RIFX") {
        let mut wave_file = std::fs::File::open(source)?;
        if let Ok(info) = fvid_media::owned_wave_inspect::inspect(&mut wave_file, cancel) {
            return fvid_media::owned_wave_inspect::info_chunks(&mut wave_file, &info, cancel)
                .map_err(|e| invalid(&e.to_string()));
        }
    }
    let base = extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
        source,
        options.streams.first().copied(),
        true,
        None,
        Some(options.max_packet_bytes),
        options.max_packets,
        options.max_controlled_bytes,
        cancel,
    )?;
    let mut extra = base.3.clone();
    if let Some(scoped) = base.4.get(&0) {
        extra.extend(scoped.clone());
    }
    let encoder = extra.get("ENCODER").map(String::as_str);
    let lang = if base.1 == "und" { "" } else { base.1.as_str() };
    fvid_media::owned_wave_metadata::info_for_file_tags(
        &fvid_media::owned_file_tags::FileTags {
            title: base.2.tags.title.clone(),
            artist: base.2.tags.artist.clone(),
            album: base.2.tags.album.clone(),
            album_artist: base.2.tags.album_artist.clone(),
            disc: base.2.tags.disc.clone(),
            publisher: base.2.tags.publisher.clone(),
            description: base.2.tags.description.clone(),
            rating: base.2.tags.rating.clone(),
            comment: base.2.tags.comment.clone(),
            date: base.2.tags.date.clone(),
            genre: base.2.tags.genre.clone(),
            copyright: base.2.tags.copyright.clone(),
            track: base.2.tags.track.clone(),
        },
        &base.0,
        lang,
        encoder,
        &extra,
    )
    .map_err(|e| invalid(&e))
}

pub(crate) fn validate_matroska_pcm_metadata(
    options: Option<&fvid_media::CopyOptions>,
) -> Result<()> {
    parse_matroska_pcm_metadata(None, options).map(|_| ())
}

pub(crate) fn check_output_memory_budget(
    source: &std::path::Path,
    channels: u16,
    stage_working_bytes: usize,
    options: &fvid_media::CopyOptions,
    matroska: Option<bool>,
) -> Result<()> {
    let Some(limit) = options.max_controlled_bytes else {
        return Ok(());
    };
    if limit == 0 {
        return Err(invalid("controlled memory budget exceeded: limit is 0"));
    }
    let check_wave = matroska != Some(true)
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty();
    let check_matroska = matroska != Some(false);
    let mut needed = stage_working_bytes.saturating_add(16 * 1024);
    if check_wave {
        let source_info_len = extract_source_wave_info(source, options)
            .map(|b| b.len())
            .unwrap_or(0);
        let assignments = options
            .metadata_set
            .iter()
            .fold(0usize, |sum, (_, v)| sum.saturating_add(v.len().saturating_add(10)));
        let wave_meta_bytes = 8usize.saturating_mul(source_info_len.saturating_add(assignments).saturating_add(12));
        needed = needed.max(
            stage_working_bytes
                .saturating_add(wave_meta_bytes)
                .saturating_add(16 * 1024),
        );
    }
    if check_matroska {
        let base = extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
            source,
            options.streams.first().copied(),
            true,
            None,
            Some(options.max_packet_bytes),
            options.max_packets,
            options.max_controlled_bytes,
            options.cancel.as_ref(),
        )
        .ok();
        let (name, language, metadata, text, scoped) =
            parse_matroska_pcm_metadata(base, Some(options))?;
        let mut tag_bytes = name.len().saturating_add(language.len()).saturating_add(64);
        for val in [
            &metadata.tags.title,
            &metadata.tags.artist,
            &metadata.tags.comment,
            &metadata.tags.date,
            &metadata.tags.album,
            &metadata.tags.genre,
            &metadata.tags.copyright,
            &metadata.tags.track,
        ] {
            if !val.is_empty() {
                tag_bytes = tag_bytes.saturating_add(val.len().saturating_add(32));
            }
        }
        for (k, v) in &text {
            tag_bytes = tag_bytes.saturating_add(k.len()).saturating_add(v.len()).saturating_add(32);
        }
        for entries in scoped.values() {
            for (k, v) in entries {
                tag_bytes = tag_bytes.saturating_add(k.len()).saturating_add(v.len()).saturating_add(32);
            }
        }
        let mka_meta_bytes = 8usize.saturating_mul(tag_bytes.saturating_add(12));
        let sink_block_bytes = usize::from(channels).saturating_mul(4096);
        needed = needed.max(
            stage_working_bytes
                .saturating_add(sink_block_bytes)
                .saturating_add(mka_meta_bytes)
                .saturating_add(16 * 1024),
        );
    }
    if needed > limit {
        return Err(invalid(&format!(
            "controlled memory budget exceeded: need {needed} bytes, limit {limit}"
        )));
    }
    Ok(())
}

fn parse_matroska_pcm_metadata(
    base: Option<MatroskaPcmMetadata>,
    options: Option<&fvid_media::CopyOptions>,
) -> Result<MatroskaPcmMetadata> {
    let (mut name, mut language, mut metadata, mut text, mut scoped) =
        base.unwrap_or_else(default_matroska_pcm_metadata);
    if let Some(o) = options {
        if o.metadata_set.len() + o.metadata_delete.len() > 64 {
            return Err(invalid("at most 64 container metadata mutations"));
        }
        if o.stream_metadata_set.len() + o.stream_metadata_delete.len() > 64 {
            return Err(invalid("at most 64 stream metadata mutations"));
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
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
            if !metadata.tags.set(key, value) && !value.is_empty() {
                text.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for key in &o.metadata_delete {
            if key.is_empty() || key.len() > 128 || key.contains('\0') {
                return Err(invalid("invalid Matroska container metadata key"));
            }
            metadata.tags.set(key, "");
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
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
                let entries = scoped.entry(0).or_default();
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if !value.is_empty() {
                    entries.insert(key.to_ascii_uppercase(), value.clone());
                }
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
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
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
    fn new_with_source_metadata(
        output: &'a mut W,
        rate: u32,
        channels: u16,
        base: Option<MatroskaPcmMetadata>,
        options: Option<&fvid_media::CopyOptions>,
    ) -> Result<Self> {
        let (name, language, metadata, text, scoped) =
            parse_matroska_pcm_metadata(base, options)?;
        let spec = [TrackSpec {
            encoding: Encoding::PcmFloat32 {
                sample_rate: rate,
                channels,
            },
            name: &name,
            language: &language,
        }];
        let frame_bytes = usize::from(channels) * 4;
        let block_bytes = frame_bytes * 1024;
        if let Some(limit) = options.and_then(|o| o.max_controlled_bytes) {
            let mut tag_bytes = name.len().saturating_add(language.len()).saturating_add(64);
            for val in [
                &metadata.tags.title,
                &metadata.tags.artist,
                &metadata.tags.comment,
                &metadata.tags.date,
                &metadata.tags.album,
                &metadata.tags.genre,
                &metadata.tags.copyright,
                &metadata.tags.track,
            ] {
                if !val.is_empty() {
                    tag_bytes = tag_bytes.saturating_add(val.len().saturating_add(32));
                }
            }
            for (k, v) in &text {
                tag_bytes = tag_bytes.saturating_add(k.len()).saturating_add(v.len()).saturating_add(32);
            }
            for entries in scoped.values() {
                for (k, v) in entries {
                    tag_bytes = tag_bytes.saturating_add(k.len()).saturating_add(v.len()).saturating_add(32);
                }
            }
            let needed = block_bytes
                .saturating_add(8usize.saturating_mul(tag_bytes.saturating_add(12)))
                .saturating_add(16 * 1024);
            if needed > limit {
                return Err(invalid(&format!(
                    "controlled memory budget exceeded: need {needed} bytes, limit {limit}"
                )));
            }
        }
        let writer =
            PacketWriter::new_with_text_metadata(output, &spec, &[], &metadata, &text, &scoped)?;
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
