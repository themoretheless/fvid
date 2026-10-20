//! The owned family-1 decoder is already qualified; transport must retain it.
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn audio(bytes: &[u8], index: usize) -> Vec<u8> {
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(bytes),
        &mut pcm,
        None,
        &fvid_control::CopyOptions {
            streams: vec![index],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.channels, 6);
    assert!(stats.sample_frames > 0);
    pcm
}
#[test]
fn hevc_export_keeps_six_channel_opus_configuration_packets_and_samples() {
    verify("hevc-opus-surround-companion.mkv");
}
#[test]
fn inferred_six_channel_opus_durations_validate_each_elementary_stream() {
    verify("hevc-opus-surround-no-duration.mkv");
}
#[test]
fn padded_elementary_stream_keeps_the_following_opus_stream_aligned() {
    let original = std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap();
    let padded = std::fs::read(root().join("hevc-opus-surround-padded-stream.mkv")).unwrap();
    assert_eq!(
        audio(&padded, 1),
        audio(&original, 1),
        "padding must not change decoded samples"
    );
    verify("hevc-opus-surround-padded-stream.mkv");
}
fn verify(file: &str) {
    let source = root().join(file);
    let bytes = std::fs::read(&source).unwrap();
    let expected = audio(&bytes, 1);
    let output = std::env::temp_dir().join(format!(
        "fvid-surround-export-{}-{file}.mkv",
        std::process::id()
    ));
    fvid_media::transcode_lossless(
        &source,
        &output,
        fvid_media::LosslessTransform {
            horizontal_flip: true,
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let data = std::fs::read(&output).unwrap();
    let mut input =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    input.scan_all().unwrap();
    let mut out =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
    out.scan_all().unwrap();
    assert_eq!(out.tracks.len(), 2);
    assert_eq!(out.tracks[1].codec, "A_OPUS");
    assert_eq!(out.tracks[1].channels, 6);
    assert_eq!(out.tracks[1].codec_private, input.tracks[1].codec_private);
    assert_eq!(out.tracks[1].codec_delay_ns, input.tracks[1].codec_delay_ns);
    assert_eq!(audio(&data, 1), expected);
    let a: Vec<_> = input
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == 2)
        .map(|(i, _)| i)
        .collect();
    let b: Vec<_> = out
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == 2)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(a.len(), 6);
    assert_eq!(a.len(), b.len());
    for (i, j) in a.into_iter().zip(b) {
        assert_eq!(input.packets[i].pts_ns, out.packets[j].pts_ns);
        assert_eq!(out.packets[j].duration_ns, Some(20_000_000));
        assert_eq!(input.read_packet(i).unwrap(), out.read_packet(j).unwrap());
    }
    let track = out.tracks[0].clone();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        track.width as usize,
        track.height as usize,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    let gold = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    let size = (track.width * track.height * 3 / 2) as usize;
    let mut frame = 0;
    for i in 0..out.packets.len() {
        if out.packets[i].track != track.number {
            continue;
        }
        let decoded = decoder.decode(&out.read_packet(i).unwrap()).unwrap();
        let raw = &gold[frame * size..(frame + 1) * size];
        let mut at = 0;
        let mut expected = Vec::new();
        for (w, h) in [
            (track.width as usize, track.height as usize),
            (track.width as usize / 2, track.height as usize / 2),
            (track.width as usize / 2, track.height as usize / 2),
        ] {
            let len = w * h;
            for row in raw[at..at + len].chunks_exact(w) {
                expected.extend(row.iter().rev().copied())
            }
            at += len;
        }
        assert_eq!(decoded.frame.data, expected);
        frame += 1;
    }
    assert_eq!(frame, 3);
    std::fs::remove_file(output).unwrap();
}

#[test]
fn discrete_channel_map_preserves_reordered_duplicated_and_silent_pcm() {
    let standard = std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap();
    let discrete = std::fs::read(root().join("hevc-opus-discrete-companion.mkv")).unwrap();
    let expected = audio(&standard, 1);
    let actual = audio(&discrete, 1);
    // Family 1 was reordered from Vorbis to WAVE; family 255 is discrete.
    // Coded targets [5,0,0,255,2,1] select WAVE [3,0,0,silent,4,1].
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(24).zip(expected.chunks_exact(24)) {
        for (channel, target) in [Some(3), Some(0), Some(0), None, Some(4), Some(1)]
            .into_iter()
            .enumerate()
        {
            let value = target.map(|c| &b[c * 4..c * 4 + 4]).unwrap_or(&[0; 4]);
            assert_eq!(&a[channel * 4..channel * 4 + 4], value);
        }
    }
    verify("hevc-opus-discrete-companion.mkv");
}

#[test]
fn explicit_family_one_map_is_applied_before_wave_speaker_permutation() {
    let discrete = audio(
        &std::fs::read(root().join("hevc-opus-discrete-companion.mkv")).unwrap(),
        1,
    );
    let remapped = audio(
        &std::fs::read(root().join("hevc-opus-remapped-surround.mkv")).unwrap(),
        1,
    );
    assert_eq!(discrete.len(), remapped.len());
    for (a, b) in remapped.chunks_exact(24).zip(discrete.chunks_exact(24)) {
        for (channel, target) in [0, 2, 1, 5, 3, 4].into_iter().enumerate() {
            assert_eq!(
                &a[channel * 4..channel * 4 + 4],
                &b[target * 4..target * 4 + 4]
            );
        }
    }
    verify("hevc-opus-remapped-surround.mkv");
}

#[test]
fn discrete_opus_wave_export_does_not_invent_speaker_positions() {
    for (name, mask) in [
        ("hevc-opus-discrete-companion.mkv", 0),
        ("hevc-opus-remapped-surround.mkv", 63),
    ] {
        let source = root().join(name);
        let output = std::env::temp_dir().join(format!(
            "fvid-discrete-wave-{}-{name}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(
            &source,
            &output,
            Default::default(),
            &fvid_control::CopyOptions {
                streams: vec![1],
                ..Default::default()
            },
        )
        .unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut pcm = vec![0; info.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        let source_bytes = std::fs::read(&source).unwrap();
        assert!(info.float);
        assert_eq!(info.bits_per_sample, 32);
        assert_eq!(
            pcm,
            audio(&source_bytes, 1),
            "export must keep discrete PCM ordering"
        );
        std::fs::remove_file(&output).unwrap();
        assert_eq!(info.channels, 6);
        assert_eq!(
            info.channel_mask, mask,
            "speaker semantics must follow the mapping family"
        );
    }
}

#[test]
fn discrete_nine_and_255_channels_preserve_pcm_through_wave_export() {
    use std::io::{Read, Seek, SeekFrom};
    let baseline = audio(
        &std::fs::read(root().join("hevc-opus-discrete-companion.mkv")).unwrap(),
        1,
    );
    for channels in [9usize, 255] {
        let source = root().join(format!("hevc-opus-discrete-{channels}.mkv"));
        let bytes = std::fs::read(&source).unwrap();
        let mut pcm = Vec::new();
        let options = fvid_control::CopyOptions {
            streams: vec![1],
            ..Default::default()
        };
        let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &options,
        )
        .unwrap();
        assert_eq!(usize::from(stats.channels), channels);
        assert_eq!(pcm.len() / channels, baseline.len() / 6);
        for (a, b) in pcm
            .chunks_exact(channels * 4)
            .zip(baseline.chunks_exact(24))
        {
            for c in 0..channels {
                assert_eq!(&a[c * 4..c * 4 + 4], &b[(c % 6) * 4..(c % 6) * 4 + 4]);
            }
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-wide-opus-{}-{channels}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(&source, &output, Default::default(), &options)
            .unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(usize::from(info.channels), channels);
        assert_eq!(info.channel_mask, 0);
        assert!(info.float);
        assert_eq!(info.bits_per_sample, 32);
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut exported = vec![0; info.data_bytes as usize];
        file.read_exact(&mut exported).unwrap();
        std::fs::remove_file(&output).unwrap();
        assert_eq!(exported, pcm, "channels={channels}");
    }
}

#[test]
fn wide_discrete_resampling_preserves_channel_independence_and_clock() {
    use std::io::{Read, Seek, SeekFrom};
    let mut baseline = Vec::new();
    for channels in [6usize, 9, 255] {
        let name = if channels == 6 {
            "hevc-opus-discrete-companion.mkv".to_string()
        } else {
            format!("hevc-opus-discrete-{channels}.mkv")
        };
        let output = std::env::temp_dir().join(format!(
            "fvid-wide-resample-{}-{channels}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(
            &root().join(name),
            &output,
            fvid_media_info::AudioDecodeTransform {
                sample_rate: Some(24000),
                ..Default::default()
            },
            &fvid_control::CopyOptions {
                streams: vec![1],
                ..Default::default()
            },
        )
        .unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(info.sample_rate, 24000);
        assert_eq!(usize::from(info.channels), channels);
        assert_eq!(info.channel_mask, 0);
        assert!(info.float);
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut pcm = vec![0; info.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        std::fs::remove_file(&output).unwrap();
        if channels == 6 {
            baseline = pcm;
        } else {
            assert_eq!(pcm.len() / channels, baseline.len() / 6);
            for (a, b) in pcm
                .chunks_exact(channels * 4)
                .zip(baseline.chunks_exact(24))
            {
                for c in 0..channels {
                    assert_eq!(&a[c * 4..c * 4 + 4], &b[(c % 6) * 4..(c % 6) * 4 + 4]);
                }
            }
        }
    }
}

#[test]
fn opus_transport_rejects_more_than_255_decoded_channels_at_identification() {
    let bytes = std::fs::read(root().join("hevc-opus-invalid-decoded-count.mkv")).unwrap();
    let reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    let head = &reader.tracks[1].codec_private;
    assert_eq!((head[19], head[20]), (254, 2));
    let error = fvid_media::owned_opus_packet::header_channels(head).unwrap_err();
    assert!(
        error.to_string().contains("more than 255 decoded channels"),
        "{error}"
    );
}

#[test]
fn discrete_encoder_source_movie_contains_the_authored_pcm_used_by_core_tests() {
    let bytes = std::fs::read(root().join("hevc-opus-discrete-encoder-source.mkv")).unwrap();
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        Cursor::new(&bytes),
        &mut pcm,
        None,
        &fvid_control::CopyOptions {
            streams: vec![1],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_rate, 48000);
    assert_eq!(stats.channels, 1);
    assert_eq!(stats.sample_frames, 2880);
    assert_eq!(
        pcm,
        std::fs::read(root().join("opus-discrete-encoder-source.f32")).unwrap()
    );
}

#[test]
fn synthetic_video_packet_cannot_publish_inconsistent_opus_stream_timing_to_ogg() {
    let bytes = std::fs::read(root().join("hevc-opus-invalid-stream-duration.mkv")).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let head = fvid_opus::OpusHead::parse(&reader.tracks[1].codec_private).unwrap();
    assert_eq!(head.stream_count, 4);
    let index = reader.packets.iter().position(|p| p.track == 2).unwrap();
    let packet = reader.read_packet(index).unwrap();
    let mut writer = fvid_opus::OggOpusWriter::new(Vec::new(), head).unwrap();
    let before = writer.get_ref().unwrap().clone();
    for explicit in [false, true] {
        let result = if explicit {
            writer.write_packet_with_duration(&packet, 960)
        } else {
            writer.write_packet(&packet)
        };
        let error = result.unwrap_err();
        assert!(error.to_string().contains("durations disagree"), "{error}");
        assert_eq!(writer.granule(), 0);
        assert_eq!(writer.get_ref().unwrap(), &before);
    }
}

#[test]
fn synthetic_surround_packet_allows_final_trim_but_not_clock_extension() {
    let bytes = std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let head = fvid_opus::OpusHead::parse(&reader.tracks[1].codec_private).unwrap();
    let index = reader.packets.iter().position(|p| p.track == 2).unwrap();
    let packet = reader.read_packet(index).unwrap();
    assert_eq!(
        fvid_opus::multistream_packet_samples(&packet, 4).unwrap(),
        960
    );
    let mut writer = fvid_opus::OggOpusWriter::new(Vec::new(), head).unwrap();
    let error = writer.write_packet_with_duration(&packet, 961).unwrap_err();
    assert!(error.to_string().contains("exceeds coded"), "{error}");
    assert_eq!(writer.granule(), 0);
    writer.write_packet_with_duration(&packet, 943).unwrap();
    let error = writer.write_packet(&packet).unwrap_err();
    assert!(
        error.to_string().contains("trimmed final packet"),
        "{error}"
    );
    let mut output = fvid_opus::OggOpusReader::new(Cursor::new(writer.finish().unwrap())).unwrap();
    let last = output.read_packet().unwrap().unwrap();
    assert_eq!(last.page_granule, 943);
    assert_eq!(last.data, packet);
    assert!(last.end_of_stream);
    assert!(output.read_packet().unwrap().is_none());
}

#[test]
fn opus_quad_wave_export_uses_rear_pair_instead_of_center_speakers() {
    use std::io::{Read, Seek, SeekFrom};
    let source = root().join("hevc-opus-quad-companion.mkv");
    let output = std::env::temp_dir().join(format!("fvid-opus-quad-{}.wav", std::process::id()));
    let six = audio(
        &std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap(),
        1,
    );
    fvid_media::decode_audio_transformed(
        &source,
        &output,
        Default::default(),
        &fvid_control::CopyOptions {
            streams: vec![1],
            ..Default::default()
        },
    )
    .unwrap();
    let mut file = std::fs::File::open(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    assert_eq!(info.channels, 4);
    assert!(info.float);
    assert_eq!(info.bits_per_sample, 32);
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut actual = vec![0; info.data_bytes as usize];
    file.read_exact(&mut actual).unwrap();
    std::fs::remove_file(&output).unwrap();
    let expected: Vec<u8> = six
        .chunks_exact(24)
        .flat_map(|row| {
            [0usize, 1, 4, 5]
                .into_iter()
                .flat_map(move |c| row[c * 4..c * 4 + 4].iter().copied())
        })
        .collect();
    assert_eq!(
        actual, expected,
        "quad fixture must preserve the two coupled streams"
    );
    assert_eq!(info.channel_mask, 0x33, "Opus quad declares FL/FR/BL/BR");
}

#[test]
fn opus_quad_rematrix_uses_both_rear_channels_in_stereo_and_mono() {
    use std::io::{Read, Seek, SeekFrom};
    let source = root().join("hevc-opus-quad-companion.mkv");
    let bytes = std::fs::read(&source).unwrap();
    let mut quad = Vec::new();
    let options = fvid_control::CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(bytes),
        &mut quad,
        None,
        &options,
    )
    .unwrap();
    for channels in [2usize, 1] {
        let output = std::env::temp_dir().join(format!(
            "fvid-quad-rematrix-{}-{channels}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(
            &source,
            &output,
            fvid_media_info::AudioDecodeTransform {
                channels: Some(channels as i32),
                ..Default::default()
            },
            &options,
        )
        .unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(usize::from(info.channels), channels);
        assert!(info.float);
        assert_eq!(info.bits_per_sample, 32);
        assert_eq!(info.channel_mask, if channels == 2 { 3 } else { 4 });
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut actual = vec![0; info.data_bytes as usize];
        file.read_exact(&mut actual).unwrap();
        std::fs::remove_file(&output).unwrap();
        let mut expected = Vec::new();
        for row in quad.chunks_exact(16) {
            let values: Vec<_> = row
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let left = values[0] + std::f32::consts::FRAC_1_SQRT_2 * values[2];
            let right = values[1] + std::f32::consts::FRAC_1_SQRT_2 * values[3];
            if channels == 2 {
                expected.extend(left.to_le_bytes());
                expected.extend(right.to_le_bytes());
            } else {
                expected.extend(((left + right) * 0.5).to_le_bytes());
            }
        }
        assert_eq!(actual, expected, "quad to {channels} channels");
    }
}

#[test]
fn quad_wave_rematrix_retains_float32_and_float64_precision() {
    use std::io::{Read, Seek, SeekFrom};
    for bits in [32usize, 64] {
        let source = root().join(format!("quad-rematrix-float{bits}.wav"));
        let mut file = std::fs::File::open(&source).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut input = vec![0; info.data_bytes as usize];
        file.read_exact(&mut input).unwrap();
        for channels in [1usize, 2] {
            let output = std::env::temp_dir().join(format!(
                "fvid-quad-precision-{}-{bits}-{channels}.wav",
                std::process::id()
            ));
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    channels: Some(channels as i32),
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let mut file = std::fs::File::open(&output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(info.float);
            assert_eq!(usize::from(info.bits_per_sample), bits);
            assert_eq!(usize::from(info.channels), channels);
            assert_eq!(info.channel_mask, if channels == 1 { 4 } else { 3 });
            assert_eq!(info.sample_frames, 2880);
            file.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut actual = vec![0; info.data_bytes as usize];
            file.read_exact(&mut actual).unwrap();
            std::fs::remove_file(&output).unwrap();
            let mut expected = Vec::new();
            for row in input.chunks_exact(bits / 8 * 4) {
                if bits == 32 {
                    let v: Vec<_> = row
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                        .collect();
                    let l = v[0] + std::f32::consts::FRAC_1_SQRT_2 * v[2];
                    let r = v[1] + std::f32::consts::FRAC_1_SQRT_2 * v[3];
                    if channels == 2 {
                        expected.extend(l.to_le_bytes());
                        expected.extend(r.to_le_bytes());
                    } else {
                        expected.extend(((l + r) * 0.5).to_le_bytes());
                    }
                } else {
                    let v: Vec<_> = row
                        .chunks_exact(8)
                        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                        .collect();
                    let l = v[0] + std::f64::consts::FRAC_1_SQRT_2 * v[2];
                    let r = v[1] + std::f64::consts::FRAC_1_SQRT_2 * v[3];
                    if channels == 2 {
                        expected.extend(l.to_le_bytes());
                        expected.extend(r.to_le_bytes());
                    } else {
                        expected.extend(((l + r) * 0.5).to_le_bytes());
                    }
                }
            }
            assert_eq!(actual, expected, "bits={bits} channels={channels}");
        }
    }
}

#[test]
fn owned_ogg_library_parser_preserves_video_companion_packets_and_logical_streams() {
    use fvid_media::owned_ogg::{Coding, Limits, Ogg};
    let video = std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap();
    let mut mkv =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&video), Default::default()).unwrap();
    mkv.scan_all().unwrap();
    let bytes = std::fs::read(root().join("opus-surround-presentation.opus")).unwrap();
    let ogg = Ogg::parse(&bytes, &Default::default()).unwrap();
    assert_eq!(ogg.pages(), 8);
    assert_eq!(ogg.chains().len(), 1);
    let chain = &ogg.chains()[0];
    assert_eq!(chain.coding, Coding::Opus);
    assert!(chain.ended);
    assert_eq!((chain.first_page, chain.last_page), (1, 8));
    assert_eq!(chain.packets[0].data, mkv.tracks[1].codec_private);
    let indices: Vec<_> = mkv
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == 2)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(chain.packets.len() - 2, indices.len());
    for (index, mkv_index) in indices.into_iter().enumerate() {
        assert_eq!(
            chain.packets[index + 2].data,
            mkv.read_packet(mkv_index).unwrap()
        );
        assert_eq!(
            chain.packets[index + 2].end_granule,
            Some((index as i64 + 1) * 960 - if index == 5 { 137 } else { 0 })
        );
    }
    assert!(matches!(
        Ogg::parse(
            &bytes,
            &Limits {
                file_bytes: bytes.len() - 1,
                ..Default::default()
            }
        ),
        Err(fvid_control::error::Error::Invalid(_))
    ));
    assert!(matches!(
        Ogg::parse(
            &bytes,
            &Limits {
                packets: 7,
                ..Default::default()
            }
        ),
        Err(fvid_control::error::Error::Invalid(_))
    ));
    for (file, count, parallel) in [
        ("opus-surround-chained.opus", 2, false),
        ("opus-surround-parallel.opus", 2, true),
        ("opus-surround-parallel-chained.opus", 4, true),
    ] {
        let bytes = std::fs::read(root().join(file)).unwrap();
        let ogg = Ogg::parse(&bytes, &Default::default()).unwrap();
        assert_eq!(ogg.chains().len(), count);
        assert_eq!(
            ogg.chains()[0].last_page >= ogg.chains()[1].first_page,
            parallel
        );
    }
}

#[test]
fn owned_ogg_opus_pcm_decode_preserves_tracks_reconfiguration_layouts_seek_and_rewind() {
    use fvid_media::owned_ogg_opus::Program;
    let six = audio(
        &std::fs::read(root().join("hevc-opus-surround-companion.mkv")).unwrap(),
        1,
    );
    let changed = audio(
        &std::fs::read(root().join("hevc-opus-reconfigured-companion.mkv")).unwrap(),
        1,
    );
    let quad_bytes = std::fs::read(root().join("hevc-opus-quad-companion.mkv")).unwrap();
    let mut quad = Vec::new();
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(quad_bytes),
        &mut quad,
        None,
        &Default::default(),
    )
    .unwrap();
    let pad_quad = |bytes: &[u8]| {
        let mut out = Vec::new();
        for row in bytes.chunks_exact(16) {
            out.extend_from_slice(&row[..8]);
            out.extend([0; 8]);
            out.extend_from_slice(&row[8..]);
        }
        out
    };
    let first = six[..5311 * 24].to_vec();
    let mut reconfigured = first.clone();
    reconfigured.extend_from_slice(&changed[..changed.len() - 211 * 24]);
    let mut changing = pad_quad(&quad[..5311 * 16]);
    changing.extend_from_slice(&six[336 * 24..5237 * 24]);
    for (file, track, expected) in [
        ("opus-surround-presentation.opus", 0, first.clone()),
        ("opus-surround-padded-presentation.opus", 0, first),
        ("opus-surround-reconfigured.opus", 0, reconfigured),
        (
            "opus-surround-parallel.opus",
            1,
            six[336 * 24..5311 * 24].to_vec(),
        ),
        ("opus-quad-to-surround.opus", 0, changing),
    ] {
        let bytes = std::fs::read(root().join(file)).unwrap();
        let mut program =
            Program::open_track(Cursor::new(&bytes), Default::default(), track).unwrap();
        assert_eq!(program.channels(), 6);
        assert_eq!(program.output_channel_mask(), Some(0x3f));
        let total = expected.len() / 24;
        for target in [0usize, 1782, 5311.min(total), total - 1, total, 0] {
            assert_eq!(program.seek_to(target as i64), target as i64);
            let mut actual = Vec::new();
            let stats = program.decode_pcm(&mut actual).unwrap();
            assert_eq!(stats.channels, 6);
            assert_eq!(stats.sample_rate, 48000);
            assert_eq!(stats.sample_frames, (total - target) as u64);
            assert!(stats.decoded_frames > 0);
            assert_eq!(
                actual,
                expected[target * 24..],
                "{file} track={track} seek={target}"
            );
        }
        program.rewind();
        let mut actual = Vec::new();
        program.decode_pcm(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn owned_ogg_pcm_writer_errors_are_reported_and_rewind_restores_decode() {
    struct Failing;
    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "synthetic PCM sink failure",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bytes = std::fs::read(root().join("opus-surround-presentation.opus")).unwrap();
    let mut program =
        fvid_media::owned_ogg_opus::Program::open(Cursor::new(&bytes), Default::default()).unwrap();
    assert!(matches!(
        program.decode_pcm(&mut Failing),
        Err(fvid_control::error::Error::Io(_))
    ));
    program.rewind();
    let mut actual = Vec::new();
    let stats = program.decode_pcm(&mut actual).unwrap();
    assert_eq!(stats.sample_frames, 5311);
    assert_eq!(actual.len(), 5311 * 24);
}

#[test]
fn public_ogg_opus_wave_export_uses_the_owned_program_for_tracks_and_intervals() {
    use std::io::{Read, Seek, SeekFrom};
    for (name, track) in [
        ("opus-surround-presentation.opus", 0),
        ("opus-surround-parallel.opus", 1),
        ("opus-surround-reconfigured.opus", 0),
        ("opus-quad-to-surround.opus", 0),
    ] {
        let source = root().join(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut program = fvid_media::owned_ogg_opus::Program::open_track(
            Cursor::new(&bytes),
            Default::default(),
            track,
        )
        .unwrap();
        let mut pcm = Vec::new();
        program.decode_pcm(&mut pcm).unwrap();
        let mut intervals = vec![None, Some((37_123i64, 91_789i64))];
        if pcm.len() / (usize::from(program.channels()) * 4) > 9000 {
            intervals.push(Some((110_123, 171_789)));
        }
        for interval in intervals {
            let output = std::env::temp_dir().join(format!(
                "fvid-public-ogg-{}-{name}-{interval:?}.wav",
                std::process::id()
            ));
            let options = fvid_control::CopyOptions {
                streams: vec![track],
                ..Default::default()
            };
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &options,
            )
            .unwrap();
            let mut file = std::fs::File::open(&output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(info.float);
            assert_eq!(info.channels, program.channels());
            assert_eq!(
                info.channel_mask,
                program.output_channel_mask().unwrap_or(0)
            );
            file.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut actual = vec![0; info.data_bytes as usize];
            file.read_exact(&mut actual).unwrap();
            std::fs::remove_file(&output).unwrap();
            let width = usize::from(program.channels()) * 4;
            let (from, to) = interval
                .map(|(a, b)| {
                    (
                        ((a as usize) * 48000).div_ceil(1_000_000),
                        ((b as usize) * 48000).div_ceil(1_000_000),
                    )
                })
                .unwrap_or((0, pcm.len() / width));
            assert_eq!(
                actual,
                pcm[from * width..to * width],
                "{name} interval={interval:?}"
            );
        }
    }
}

#[test]
fn ogg_wave_export_controls_bound_work_and_publish_only_completed_outputs() {
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::{Arc, Mutex};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-ogg-wave-controls-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let source = root().join("opus-surround-presentation.opus");
    let bytes = std::fs::read(&source).unwrap();
    let mut program =
        fvid_media::owned_ogg_opus::Program::open(Cursor::new(&bytes), Default::default()).unwrap();
    let mut full = Vec::new();
    program.decode_pcm(&mut full).unwrap();
    for (name, options, message) in [
        (
            "memory.wav",
            fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
        (
            "packet.wav",
            fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            "packet exceeds byte limit",
        ),
    ] {
        let output = directory.0.join(name);
        let error = fvid_media::decode_audio(&source, &output, &options).unwrap_err();
        assert!(error.contains(message), "{error}");
        assert!(!output.exists());
    }
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    let output = directory.0.join("cancelled.wav");
    let error = fvid_media::decode_audio(
        &source,
        &output,
        &fvid_control::CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!output.exists());
    let cancel = fvid_control::CancelFlag::new();
    let captured = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let output = directory.0.join("during.wav");
    let error = fvid_media::decode_audio(
        &source,
        &output,
        &fvid_control::CopyOptions {
            cancel: Some(cancel),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event);
                if event.packets >= 1 {
                    captured.cancel();
                }
            })),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!output.exists());
    assert!(!events.lock().unwrap().iter().any(|e| e.done));
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let output = directory.0.join("prefix.wav");
    let stats = fvid_media::decode_audio(
        &source,
        &output,
        &fvid_control::CopyOptions {
            max_packets: Some(3),
            max_controlled_bytes: Some(64 * 1024 * 1024),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event)
            })),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.decoded_frames, 3);
    assert_eq!(stats.sample_frames, 2568);
    let mut file = std::fs::File::open(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut pcm = vec![0; info.data_bytes as usize];
    file.read_exact(&mut pcm).unwrap();
    assert_eq!(pcm, full[..2568 * 24]);
    let events = events.lock().unwrap();
    assert!(events.last().unwrap().done);
    assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    drop(events);
    let existing = directory.0.join("existing.wav");
    std::fs::write(&existing, b"preserve existing bytes").unwrap();
    assert!(fvid_media::decode_audio(&source, &existing, &Default::default()).is_err());
    assert_eq!(std::fs::read(existing).unwrap(), b"preserve existing bytes");
    let parallel = root().join("opus-surround-parallel.opus");
    assert!(fvid_media::decode_audio(
        &parallel,
        &directory.0.join("ambiguous.wav"),
        &Default::default()
    )
    .unwrap_err()
    .contains("select exactly one"));
}

#[test]
fn ogg_discrete_wave_export_preserves_tags_without_inventing_speakers() {
    let source = root().join("opus-discrete-tagged.opus");
    let output =
        std::env::temp_dir().join(format!("fvid-discrete-tags-{}.wav", std::process::id()));
    fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
    let mut file = std::fs::File::open(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    let chunks = fvid_media::owned_wave_inspect::info_chunks(&mut file, &info, None).unwrap();
    std::fs::remove_file(&output).unwrap();
    assert_eq!(info.channels, 6);
    assert_eq!(info.channel_mask, 0);
    assert!(
        chunks
            .windows(b"Discrete program".len())
            .any(|w| w == b"Discrete program"),
        "source title was lost: {chunks:?}"
    );
}

#[test]
fn ogg_wave_dsp_and_metadata_edits_match_the_decoded_wave_workflow() {
    use std::io::{Read, Seek, SeekFrom};
    let directory = std::env::temp_dir().join(format!("fvid-ogg-dsp-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = root().join("opus-surround-parallel.opus");
    let baseline = directory.join("baseline.wav");
    let selection = fvid_control::CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    fvid_media::decode_audio(&source, &baseline, &selection).unwrap();
    for rate in [32000, 44100, 96000] {
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((37_123, 91_789)),
            sample_rate: Some(rate),
            channels: Some(2),
            volume: Some(0.375),
        };
        let actual = directory.join(format!("ogg-{rate}.wav"));
        let expected = directory.join(format!("wave-{rate}.wav"));
        let edits = fvid_control::CopyOptions {
            streams: vec![1],
            metadata_set: vec![("title".into(), "Edited title".into())],
            metadata_delete: vec!["ILNG".into()],
            ..Default::default()
        };
        fvid_media::decode_audio_transformed(&source, &actual, transform, &edits).unwrap();
        let wave_edits = fvid_control::CopyOptions {
            streams: vec![0],
            ..edits
        };
        fvid_media::decode_audio_transformed(&baseline, &expected, transform, &wave_edits).unwrap();
        let read = |path: &std::path::Path| {
            let mut file = std::fs::File::open(path).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert_eq!(info.sample_rate, rate as u32);
            assert_eq!(info.channels, 2);
            let tags = fvid_media::owned_wave_inspect::info_chunks(&mut file, &info, None).unwrap();
            assert!(tags.windows(12).any(|s| s == b"Edited title"));
            assert!(!tags.windows(4).any(|s| s == b"ILNG"));
            file.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut pcm = vec![0; info.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            (pcm, tags)
        };
        assert_eq!(read(&actual), read(&expected), "rate={rate}");
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
