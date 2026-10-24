#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioPacket, AudioStep, AudioStream, AudioTrack, EncodedPacket},
    codec::{
        ac3_decoder::{Ac3Decoder, CompressionMode},
        eac3_decoder::{presentation::PresentationDecoder, Eac3Decoder},
    },
};
use std::{io::Cursor, path::Path, time::Duration};
use support::DecodeComplete;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/dolby-player-normalization")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> Box<dyn AudioStream> {
    if n.ends_with(".mp4") {
        Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    } else {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    }
}
fn reference(n: &str, mode: CompressionMode, normalize: bool) -> Vec<(Duration, Vec<u8>)> {
    reference_seek(n, mode, normalize, None)
}
fn reference_seek(
    n: &str,
    mode: CompressionMode,
    normalize: bool,
    seek: Option<Duration>,
) -> Vec<(Duration, Vec<u8>)> {
    let mut s = open(n);
    if let Some(seek) = seek {
        let ticks = (seek.as_nanos() * u128::from(s.timescale()) / 1_000_000_000) as i64;
        s.seek_to(ticks);
    }
    let mut d: Box<dyn AudioDecode> = if matches!(s.codec(), "A_AC3" | "ac-3") {
        Box::new(
            Ac3Decoder::for_container_with_compression(
                s.extra_data(),
                s.sample_rate(),
                s.channels(),
                mode,
            )
            .unwrap()
            .with_dialogue_normalization(normalize),
        )
    } else if matches!(s.codec(), "A_EAC3" | "ec-3") {
        Box::new(
            Eac3Decoder::for_container_with_compression(
                s.extra_data(),
                s.sample_rate(),
                s.channels(),
                mode,
            )
            .unwrap()
            .with_dialogue_normalization(normalize),
        )
    } else {
        s.make_decoder().unwrap()
    };
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Pcm(p) = step {
            out.push((p.presentation_time(), p.data));
            continue;
        }
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    out.extend(
        d.drain_presented(&mut *s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    out
}
struct Replacing {
    inner: Box<dyn AudioStream>,
    pending_reset: bool,
    direct: bool,
}
impl AudioStream for Replacing {
    fn codec(&self) -> &str {
        self.inner.codec()
    }
    fn timescale(&self) -> u32 {
        self.inner.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn bits_per_sample(&self) -> u16 {
        self.inner.bits_per_sample()
    }
    fn duration(&self) -> Option<Duration> {
        self.inner.duration()
    }
    fn extra_data(&self) -> &[u8] {
        self.inner.extra_data()
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.inner.audio_tracks()
    }
    fn speaker_mask(&self) -> fvid::Result<Option<u32>> {
        self.inner.speaker_mask()
    }
    fn make_decoder(&self) -> fvid::Result<Box<dyn AudioDecode>> {
        if self.direct {
            Ok(Box::new(PresentationDecoder::for_container(
                self.sample_rate(),
                self.channels(),
            )?))
        } else {
            self.inner.make_decoder()
        }
    }
    fn reset_decoder(&mut self, d: &mut Box<dyn AudioDecode>) -> fvid::Result<()> {
        *d = self.make_decoder()?;
        Ok(())
    }
    fn next_packet(&mut self) -> fvid::Result<Option<EncodedPacket>> {
        self.inner.next_packet()
    }
    fn next_step(&mut self) -> fvid::Result<Option<AudioStep>> {
        if self.pending_reset {
            self.pending_reset = false;
            Ok(Some(AudioStep::ResetDecoder))
        } else {
            self.inner.next_step()
        }
    }
    fn rewind(&mut self) {
        self.inner.rewind();
        self.pending_reset = true;
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.pending_reset = true;
        self.inner.seek_to(pts)
    }
    fn present_decoded(&self, p: AudioPacket, pts: i64) -> fvid::Result<Option<AudioPacket>> {
        self.inner.present_decoded(p, pts)
    }
    fn packet_sample_limit(&self, d: u64) -> fvid::Result<Option<usize>> {
        self.inner.packet_sample_limit(d)
    }
    fn validate_eof(&self) -> fvid::Result<()> {
        self.inner.validate_eof()
    }
    fn preroll_target(&self) -> Option<i64> {
        self.inner.preroll_target()
    }
    fn resume_preroll(&mut self, p: i64) -> bool {
        self.inner.resume_preroll(p)
    }
}
#[test]
fn worker_dialogue_normalization_survives_navigation_and_reader_decoder_replacement() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for c in cases {
        let file = c["file"].as_str().unwrap();
        let width = open(file).channels() as usize;
        let rate = u64::from(open(file).sample_rate());
        let seek = Duration::from_millis(c["seek_ms"].as_u64().unwrap());
        assert_eq!(
            open(file).speaker_mask().unwrap(),
            c["mask"].as_u64().map(|v| v as u32)
        );
        for mode in [CompressionMode::Line, CompressionMode::Rf] {
            let plain = reference(file, mode, false);
            let normalized = reference(file, mode, true);
            if c["dolby"] == false {
                assert_eq!(plain, normalized);
                assert!(plain.iter().flat_map(|(_, b)| b).any(|b| *b != 0));
            } else {
                assert_ne!(
                    plain, normalized,
                    "{file}: distinguish normalization policies"
                );
            }
            for normalize in [true, false] {
                let expected = if normalize { &normalized } else { &plain };
                let mut skipped = (rate * seek.as_millis() as u64 / 1000) as usize;
                let mut wanted = Vec::new();
                for (at, b) in expected {
                    let frames = b.len() / (width * 4);
                    if skipped >= frames {
                        skipped -= frames;
                        continue;
                    }
                    let mut data = b.clone();
                    let pts = if wanted.is_empty() {
                        data.drain(..skipped * width * 4);
                        skipped = 0;
                        seek
                    } else {
                        *at
                    };
                    wanted.push((pts, data));
                }
                if c["dolby"] == false {
                    wanted = reference_seek(file, mode, normalize, Some(seek));
                }
                for replace in [false, true] {
                    for direct in if replace && width > 6 {
                        vec![false, true]
                    } else {
                        vec![false]
                    } {
                        let audio: Box<dyn AudioStream> = if replace {
                            Box::new(Replacing {
                                inner: open(file),
                                pending_reset: true,
                                direct,
                            })
                        } else {
                            open(file)
                        };
                        capture::worker_check_with_dolby_policy(
                            audio,
                            expected,
                            seek,
                            &wanted,
                            c["mask"].as_u64().map(|v| v as u32),
                            fvid::audio_thread::DolbyPlaybackPolicy {
                                compression: mode,
                                normalize_dialogue: normalize,
                            },
                        );
                    }
                }
            }
        }
    }
}
