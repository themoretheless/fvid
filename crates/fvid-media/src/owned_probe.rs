//! Metadata inspection through owned container parsers.
use fvid_media_info::{MediaInfo, StreamInfo};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

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
    let codec = if wave.float {
        format!("pcm_f{}le", wave.bits_per_sample)
    } else if wave.bits_per_sample == 8 {
        "pcm_u8".into()
    } else {
        format!("pcm_s{}le", wave.bits_per_sample)
    };
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
        #[cfg(feature = "legacy-ffmpeg")]
        assert_eq!(crate::probe(&path).unwrap().metadata, info.metadata);
        std::fs::remove_file(path).unwrap();
    }
}
