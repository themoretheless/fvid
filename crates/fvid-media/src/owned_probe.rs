//! Metadata inspection through owned container parsers.
use fvid_media_info::{MediaInfo, StreamInfo};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Inspect media through the owned parsers available in this library.
/// Additional container parsers are being moved here from the native frontend.
pub fn probe(path: &Path) -> Result<MediaInfo, String> {
    probe_as(path, None)
}

/// Inspect owned RIFF/WAVE PCM or AAC-LC ADTS, optionally requiring a format.
/// Format hints never override the signature or bypass container validation.
pub fn probe_as(path: &Path, format: Option<&str>) -> Result<MediaInfo, String> {
    match format {
        Some("wav") => return probe_wave(path).map_err(|e| e.to_string()),
        Some("aac") => return probe_adts(path),
        Some(_) => return Err("format has no owned media-library probe yet".into()),
        None => {}
    }
    if crate::owned_wave_inspect::is_wave(path).map_err(|e| e.to_string())? {
        return probe_wave(path).map_err(|e| e.to_string());
    }
    let mut prefix = [0; 7];
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    match file.read_exact(&mut prefix) {
        Ok(()) if crate::owned_aac::adts::header(&prefix).is_some() => probe_adts(path),
        Ok(()) => Err("container has no owned media-library probe yet".into()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err("container has no owned media-library probe yet".into())
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Count AAC-LC ADTS packets using owned framing, without decoding PCM.
/// The streaming reader retains only one bounded ADTS packet at a time.
pub fn probe_adts(path: &Path) -> Result<MediaInfo, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = crate::owned_aac::adts::StreamReader::open(std::io::BufReader::new(file))
        .map_err(|e| e.to_string())?;
    let config = reader.configuration();
    let mut frames = 0u64;
    while reader.next_packet().map_err(|e| e.to_string())?.is_some() {
        frames = frames.checked_add(1024).ok_or("AAC duration overflow")?;
    }
    let duration = i64::try_from(frames).map_err(|_| "AAC duration exceeds API range")?;
    let duration_us =
        i64::try_from(u128::from(frames) * 1_000_000 / u128::from(config.sample_rate))
            .map_err(|_| "AAC duration exceeds API range")?;
    Ok(MediaInfo {
        path: path.into(),
        format: "aac".into(),
        start_us: Some(0),
        duration_us: Some(duration_us),
        bit_rate: None,
        metadata: BTreeMap::new(),
        chapters: Vec::new(),
        streams: vec![StreamInfo {
            index: 0,
            media_type: "audio".into(),
            codec: "aac".into(),
            time_base: [1, config.sample_rate as i32],
            start: Some(0),
            duration: Some(duration),
            bit_rate: None,
            average_frame_rate: [0, 1],
            profile: Some("LC".into()),
            level: None,
            disposition: 0,
            metadata: BTreeMap::new(),
            width: 0,
            height: 0,
            pixel_format: -1,
            sample_rate: config.sample_rate as i32,
            channels: i32::from(config.channels),
            video_delay: 0,
            extradata_bytes: 2,
        }],
    })
}

/// Inspect supported uncompressed RIFF/WAVE without loading its sample payload.
pub fn probe_wave(path: &Path) -> std::io::Result<MediaInfo> {
    let mut file = File::open(path)?;
    let wave = crate::owned_wave_inspect::inspect(&mut file, None)?;
    let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
    let rate = i32::try_from(wave.sample_rate)
        .map_err(|_| invalid("WAVE sample rate exceeds metadata range"))?;
    let duration =
        i64::try_from(wave.sample_frames).map_err(|_| invalid("WAVE duration overflow"))?;
    let duration_us =
        i64::try_from(u128::from(wave.sample_frames) * 1_000_000 / u128::from(wave.sample_rate))
            .map_err(|_| invalid("WAVE duration overflow"))?;
    let mut metadata = BTreeMap::new();
    let mut at = 12;
    while at < wave.end {
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let size = u32::from_le_bytes(header[4..].try_into().unwrap());
        if &header[..4] == b"LIST" {
            let mut kind = [0; 4];
            file.read_exact(&mut kind)?;
            let mut cursor = at + 12;
            let end = at + 8 + u64::from(size);
            while cursor < end {
                if end - cursor < 8 {
                    return Err(invalid("truncated WAVE INFO entry"));
                }
                file.seek(SeekFrom::Start(cursor))?;
                file.read_exact(&mut header)?;
                let length = u32::from_le_bytes(header[4..].try_into().unwrap());
                let next = cursor + 8 + u64::from(length) + u64::from(length & 1);
                if next > end {
                    return Err(invalid("WAVE INFO entry exceeds LIST"));
                }
                // Avoid allocating unbounded metadata strings from an untrusted file.
                if length > 1_048_576 {
                    return Err(invalid("WAVE INFO entry is too large"));
                }
                let mut bytes = vec![0; length as usize];
                file.read_exact(&mut bytes)?;
                let key = match &header[..4] {
                    b"INAM" => "title",
                    b"IART" => "artist",
                    b"ICMT" => "comment",
                    b"ICRD" => "date",
                    b"IPRD" => "album",
                    b"IGNR" => "genre",
                    b"ICOP" => "copyright",
                    b"ISFT" => "encoder",
                    b"ITRK" => "track",
                    _ => std::str::from_utf8(&header[..4])
                        .map_err(|_| invalid("invalid WAVE INFO tag"))?,
                };
                let text = String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .to_owned();
                metadata.insert(key.to_owned(), text);
                cursor = next;
            }
        }
        at += 8 + u64::from(size) + u64::from(size & 1);
    }
    let codec = wave.codec();
    let bit_rate = i64::from(wave.sample_rate) * i64::from(wave.block) * 8;
    Ok(MediaInfo {
        path: path.into(),
        format: "wav".into(),
        start_us: Some(0),
        duration_us: Some(duration_us),
        bit_rate: Some(bit_rate),
        metadata,
        chapters: Vec::new(),
        streams: vec![StreamInfo {
            index: 0,
            media_type: "audio".into(),
            codec,
            time_base: [1, rate],
            start: Some(0),
            duration: Some(duration),
            bit_rate: Some(bit_rate),
            average_frame_rate: [0, 0],
            profile: None,
            level: None,
            disposition: 0,
            metadata: BTreeMap::new(),
            width: 0,
            height: 0,
            pixel_format: -1,
            sample_rate: rate,
            channels: i32::from(wave.channels),
            video_delay: 0,
            extradata_bytes: 0,
        }],
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn public_adts_probe_and_reader_use_owned_synthetic_packets() {
        let fixtures =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        for name in [
            "aac-mono-44k.aac",
            "aac-stereo.aac",
            "aac-51-active.aac",
            "aac-96k.aac",
            "aac-pce-wide8.aac",
        ] {
            let source = fixtures.join("audio").join(name);
            let bytes = std::fs::read(&source).unwrap();
            let indexed = crate::owned_aac::adts::Aac::parse(&bytes, &Default::default()).unwrap();
            let mut streamed =
                crate::owned_aac::adts::StreamReader::open(bytes.as_slice()).unwrap();
            assert_eq!(
                streamed.audio_specific_config(),
                indexed.configuration.as_slice()
            );
            for index in 0..indexed.packets() {
                assert_eq!(
                    streamed.next_packet().unwrap().unwrap(),
                    indexed.packet(index)
                );
            }
            assert!(streamed.next_packet().unwrap().is_none());
            let expected = super::probe_adts(&source).unwrap();
            assert_eq!(expected.streams[0].duration, Some(indexed.samples() as i64));
            for actual in [
                crate::probe(&source).unwrap(),
                crate::probe_as(&source, Some("aac")).unwrap(),
            ] {
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
            }
        }
        let truncated = fixtures.join("playback-errors/aac-packet-prefix.aac");
        assert!(
            super::probe_adts(&truncated)
                .unwrap_err()
                .contains("fill whole buffer")
        );
        let mono = fixtures.join("audio/aac-mono-44k.aac");
        let info = super::probe_adts(&mono).unwrap();
        assert_eq!(
            (
                info.streams[0].duration,
                info.streams[0].channels,
                info.streams[0].sample_rate
            ),
            (Some(7168), 1, 44100)
        );
    }

    #[test]
    fn public_probe_preserves_integer_storage_and_valid_bits() {
        let dir = std::env::temp_dir().join(format!("fvid-probe-precision-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for bits in [8, 16, 24, 32] {
            let path = dir.join(format!("pcm-{bits}.wav"));
            let _ = std::fs::remove_file(&path);
            crate::owned_wav_file::write_wav_integer_le(
                &path,
                44100,
                2,
                bits,
                &vec![0; 441 * 2 * usize::from(bits / 8)],
                3,
            )
            .unwrap();
            if bits == 32 {
                // Extensible PCM stores 24 valid bits in a 32-bit word.
                let mut bytes = std::fs::read(&path).unwrap();
                assert_eq!(&bytes[12..16], b"fmt ");
                assert_eq!(
                    u16::from_le_bytes(bytes[20..22].try_into().unwrap()),
                    0xfffe
                );
                bytes[38..40].copy_from_slice(&24u16.to_le_bytes());
                std::fs::write(&path, bytes).unwrap();
            }
            let expected = super::probe_wave(&path).unwrap();
            assert_eq!(expected.duration_us, Some(10000));
            assert_eq!(expected.streams[0].duration, Some(441));
            assert_eq!(expected.streams[0].channels, 2);
            assert_eq!(
                expected.streams[0].bit_rate,
                Some(44100 * 2 * i64::from(bits))
            );
            assert_eq!(
                expected.streams[0].codec,
                if bits == 8 {
                    "pcm_u8".into()
                } else {
                    format!("pcm_s{bits}le")
                }
            );
            for actual in [
                crate::probe(&path).unwrap(),
                crate::probe_as(&path, Some("wav")).unwrap(),
            ] {
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
            }
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn own_float_wave_probe() {
        let path =
            std::env::temp_dir().join(format!("fvid-own-wave-probe-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&path);
        crate::owned_wav_file::write_wav_f32le(&path, 48000, 2, &[0.0; 960]).unwrap();
        let info = super::probe_wave(&path).unwrap();

        assert_eq!(info.duration_us, Some(10000));
        assert_eq!(info.streams[0].duration, Some(480));
        assert_eq!(info.streams[0].codec, "pcm_f32le");
        assert_eq!(info.streams[0].bit_rate, Some(3072000));
        // An INFO list after the audio payload must survive native probing.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&18u32.to_le_bytes());
        bytes.extend_from_slice(b"INFOINAM");
        bytes.extend_from_slice(&6u32.to_le_bytes());
        bytes.extend_from_slice(b"test!\0");
        let size = u32::try_from(bytes.len() - 8).unwrap();
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let info = super::probe_wave(&path).unwrap();
        assert_eq!(
            info.metadata.get("title").map(String::as_str),
            Some("test!")
        );
        for actual in [
            crate::probe(&path).unwrap(),
            crate::probe_as(&path, Some("wav")).unwrap(),
            super::probe_as(&path, None).unwrap(),
        ] {
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(&info).unwrap()
            );
        }
        assert!(super::probe_as(&path, Some("aac")).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
