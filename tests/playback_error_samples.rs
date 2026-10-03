//! Synthetic reproductions of failures found in the local playback audit.
use fvid::{
    container::mp4::{Limits, Mp4Reader},
    playback_native::{NativeReader, RawFrame},
};
use std::{io::Cursor, time::Duration};

const CONTROL: &[u8] = include_bytes!("fixtures/playback-errors/control.mp4");
const GAP: &[u8] = include_bytes!("fixtures/playback-errors/edit-gap.mov");
const THREE: &[u8] = include_bytes!("fixtures/playback-errors/edit-three-ranges.mov");
const REPEAT: &[u8] = include_bytes!("fixtures/playback-errors/edit-repeat.mov");
const AVCC: &[u8] = include_bytes!("fixtures/playback-errors/avcc-invalid-reserved.mp4");
const DUPLICATE: &[u8] = include_bytes!("fixtures/playback-errors/duplicate-pts.mp4");
const DUPLICATE_RUN: &[u8] = include_bytes!("fixtures/playback-errors/duplicate-pts-run.mp4");

fn frames(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut reader =
        NativeReader::software(Cursor::new(data), 16 << 20).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().map_err(|e| e.to_string())? {
        frames.push(pixels(frame));
        assert!(frames.len() <= 24, "unexpected frame count");
    }
    Ok(frames)
}

fn pixels(frame: RawFrame) -> Vec<u8> {
    match frame {
        RawFrame::Avc { picture, .. } => {
            let mut bytes = Vec::new();
            picture.write_planar(&mut bytes).unwrap();
            bytes
        }
        RawFrame::Planar8(p) => [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()].concat(),
        _ => panic!("expected 8-bit software frame"),
    }
}

fn edits(data: &[u8]) -> Vec<(u64, i64)> {
    let reader = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    assert_eq!(reader.movie_timescale(), 12000);
    assert_eq!(reader.tracks()[0].timescale, 12000);
    reader.tracks()[0]
        .edits
        .iter()
        .map(|e| (e.duration, e.media_time))
        .collect()
}

#[test]
fn control_decodes_twelve_frames() {
    let frames = frames(CONTROL).unwrap();
    assert_eq!(frames.len(), 12);
    assert!(
        frames.windows(2).any(|w| w[0] != w[1]),
        "control needs motion"
    );
}

#[test]
fn reproductions_keep_the_controls_encoded_pictures() {
    let mut control = Mp4Reader::open(Cursor::new(CONTROL), Limits::default()).unwrap();
    for data in [GAP, THREE, REPEAT, AVCC, DUPLICATE, DUPLICATE_RUN] {
        let mut reader = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        assert_eq!(reader.tracks()[0].samples.len(), 12);
        for i in 0..12 {
            let mut expected = Vec::new();
            let mut actual = Vec::new();
            control.read_packet(0, i, &mut expected).unwrap();
            reader.read_packet(0, i, &mut actual).unwrap();
            assert_eq!(actual, expected, "encoded picture {i}");
        }
    }
}

#[test]
fn two_ranges_describe_media_gap() {
    assert_eq!(edits(GAP), [(2000, 0), (8000, 4000)]);
    assert_eq!(frames(GAP).unwrap().len(), 10);
}

#[test]
fn three_ranges_describe_two_media_gaps() {
    assert_eq!(edits(THREE), [(2000, 0), (2000, 4000), (4000, 8000)]);
    assert_eq!(frames(THREE).unwrap().len(), 8);
}

#[test]
fn repeated_range_describes_media_overlap() {
    assert_eq!(edits(REPEAT), [(1000, 0), (12000, 0)]);
    assert_eq!(frames(REPEAT).unwrap().len(), 13);
}

#[test]
fn malformed_avcc_preserves_parameter_sets_and_decodes() {
    let reader = Mp4Reader::open(Cursor::new(AVCC), Limits::default()).unwrap();
    assert!(
        reader.tracks()[0]
            .configuration
            .ends_with(&[0x7b, 0xf7, 0xf7, 0])
    );
    assert_eq!(frames(AVCC).unwrap(), frames(CONTROL).unwrap());
}

#[test]
fn compatible_avcc_still_rejects_truncation_and_invalid_extension_nals() {
    use fvid::codec::config::AvcConfig;
    let reader = Mp4Reader::open(Cursor::new(AVCC), Limits::default()).unwrap();
    let config = &reader.tracks()[0].configuration;
    for missing in 1..=3 {
        assert!(AvcConfig::parse(&config[..config.len() - missing]).is_err());
    }
    let mut malformed = config.clone();
    *malformed.last_mut().unwrap() = 1;
    assert!(
        AvcConfig::parse(&malformed).is_err(),
        "missing extension NAL"
    );
    malformed.extend_from_slice(&[0, 1, 12]);
    assert!(
        AvcConfig::parse(&malformed).is_err(),
        "wrong extension NAL type"
    );
    let mut extra = config.clone();
    extra.push(0);
    assert!(
        AvcConfig::parse(&extra).is_err(),
        "unexpected trailing bytes"
    );
}

#[test]
fn duplicate_pts_has_valid_dts_and_keeps_last_picture_at_each_time() {
    let reader = Mp4Reader::open(Cursor::new(DUPLICATE), Limits::default()).unwrap();
    let samples = &reader.tracks()[0].samples;
    assert_eq!(samples.len(), 12);
    assert_eq!(samples.get(5).unwrap().pts, samples.get(6).unwrap().pts);
    for i in 1..12 {
        assert!(samples.get(i).unwrap().dts > samples.get(i - 1).unwrap().dts);
    }
    let control = frames(CONTROL).unwrap();
    let mapping = [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11];
    assert_eq!(
        frames(DUPLICATE).unwrap(),
        mapping
            .iter()
            .map(|&i| control[i].clone())
            .collect::<Vec<_>>()
    );
    let mut reader = NativeReader::software(Cursor::new(DUPLICATE), 16 << 20).unwrap();
    for pass in 0..2 {
        let mut previous = 0;
        for &index in &mapping {
            assert_eq!(
                pixels(reader.read_frame_raw().unwrap().unwrap()),
                control[index],
                "pass {pass}"
            );
            let (start, end, scale) = reader.frame_interval().unwrap();
            assert_eq!(start, previous);
            assert!(end > start);
            assert_eq!(scale, 12000);
            previous = end;
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind().unwrap();
    }
    for millis in [417, 500, 582] {
        assert_eq!(
            pixels(
                reader
                    .seek_raw(Duration::from_millis(millis))
                    .unwrap()
                    .unwrap()
            ),
            control[6]
        );
    }
}

#[test]
fn a_run_of_equal_pts_keeps_the_last_picture_without_moving_later_frames() {
    let control = frames(CONTROL).unwrap();
    let mapping = [0, 1, 2, 7, 8, 9, 10, 11];
    assert_eq!(
        frames(DUPLICATE_RUN).unwrap(),
        mapping
            .iter()
            .map(|&i| control[i].clone())
            .collect::<Vec<_>>()
    );
    let mut reader = NativeReader::software(Cursor::new(DUPLICATE_RUN), 16 << 20).unwrap();
    for millis in [250, 400, 666] {
        assert_eq!(
            pixels(
                reader
                    .seek_raw(Duration::from_millis(millis))
                    .unwrap()
                    .unwrap()
            ),
            control[7]
        );
        assert_eq!(reader.frame_interval(), Some((3000, 8000, 12000)));
    }
}

#[test]
fn duplicate_pts_at_eof_retains_duration_and_the_final_picture() {
    let control = frames(CONTROL).unwrap();
    for data in [
        include_bytes!("fixtures/playback-errors/duplicate-pts-tail.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/duplicate-pts-all.mp4").as_slice(),
    ] {
        let mut reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
        while reader.read_frame_raw().unwrap().is_some() {}
        assert_eq!(reader.frame_interval().unwrap().1, 12000);
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        assert_eq!(
            pixels(
                reader
                    .seek_raw(Duration::from_millis(999))
                    .unwrap()
                    .unwrap()
            ),
            control[11]
        );
        let (start, end, scale) = reader.frame_interval().unwrap();
        assert!(start * 1000 <= 999 * u128::from(scale));
        assert!(999 * u128::from(scale) < end * 1000);
    }
}

// Acceptance gates for pixel selection and movie-time mapping.
fn check_mapping(data: &[u8], mapping: &[usize]) {
    let control = frames(CONTROL).unwrap();
    let expected: Vec<_> = mapping.iter().map(|&i| control[i].clone()).collect();
    assert_eq!(frames(data).unwrap(), expected);
    let mut reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
    assert_eq!(
        reader.duration(),
        Some(Duration::from_nanos(
            mapping.len() as u64 * 1_000_000_000 / 12
        ))
    );
    let cached = reader.cache_packets();
    assert_eq!(cached.len(), mapping.len());
    for (i, expected) in expected.iter().enumerate() {
        assert_eq!(pixels(reader.read_frame_raw().unwrap().unwrap()), *expected);
        assert_eq!(
            reader.frame_interval().unwrap(),
            (i as u128 * 1000, (i as u128 + 1) * 1000, 12000),
            "movie timestamps at frame {i}"
        );
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    for &i in &[0, mapping.len() / 2, mapping.len() - 1] {
        let frame = reader
            .seek_raw(Duration::from_nanos(i as u64 * 1_000_000_000 / 12 + 1))
            .unwrap()
            .unwrap();
        assert_eq!(pixels(frame), expected[i], "seek to movie frame {i}");
    }
    reader.rewind().unwrap();
    assert_eq!(
        pixels(reader.read_frame_raw().unwrap().unwrap()),
        expected[0]
    );
    assert_eq!(
        reader.cache_packets(),
        cached,
        "cache timeline must not depend on the active edit"
    );
}

#[test]
fn acceptance_gap_skips_media_and_seeks_on_movie_timeline() {
    check_mapping(GAP, &[0, 1, 4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn acceptance_three_ranges_skip_both_media_gaps() {
    check_mapping(THREE, &[0, 1, 4, 5, 8, 9, 10, 11]);
}

#[test]
fn acceptance_repeat_replays_first_frame_and_seeks_on_movie_timeline() {
    check_mapping(REPEAT, &[0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn edits_clip_partial_frames_and_restart_dependent_picture_decode() {
    let control = frames(include_bytes!(
        "fixtures/playback-errors/control-inter-frames.mp4"
    ))
    .unwrap();
    let data = include_bytes!("fixtures/playback-errors/edit-gap-inter-frames.mov");
    let mapping = [0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    let ends = [
        1000u128, 1500, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000,
    ];
    let mut reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
    assert_eq!(reader.duration(), Some(Duration::from_nanos(833333333)));
    let mut start = 0;
    for (i, end) in ends.iter().enumerate() {
        assert_eq!(
            pixels(reader.read_frame_raw().unwrap().unwrap()),
            control[mapping[i]]
        );
        assert_eq!(reader.frame_interval(), Some((start, *end, 12000)));
        start = *end;
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    for ticks in [0, 999, 1000, 1499, 1500, 1999, 2000, 9999, 12000] {
        let index = ends
            .iter()
            .position(|&end| end > ticks)
            .unwrap_or(ends.len() - 1);
        let target = Duration::from_nanos((ticks as u64 * 1_000_000_000).div_ceil(12000));
        assert_eq!(
            pixels(reader.seek_raw(target).unwrap().unwrap()),
            control[mapping[index]],
            "seek {ticks}"
        );
    }
    let cached = reader.cache_packets();
    assert!(
        cached
            .iter()
            .all(|p| p.3 <= reader.duration().unwrap() + Duration::from_nanos(1))
    );
}

#[test]
fn acceptance_reserved_bits_do_not_change_decoded_pixels() {
    check_mapping(AVCC, &(0..12).collect::<Vec<_>>());
}

#[cfg(feature = "player")]
#[test]
fn quicktime_aac_and_stale_channels_present_the_same_pcm() {
    use fvid::audio::AudioStream;
    fn decode(bytes: &[u8]) -> Vec<u8> {
        let mut stream =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Limits::default())
                .unwrap();
        assert_eq!((stream.sample_rate(), stream.channels()), (48000, 1));
        let mut decoder = fvid::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .unwrap();
        let mut output = Vec::new();
        while let Some(packet) = stream.next_packet().unwrap() {
            if let Some(mut pcm) = decoder
                .decode_encoded(
                    &packet.data,
                    packet.pts.max(0) as u64,
                    packet.duration as u64,
                )
                .unwrap()
            {
                if let Some(limit) = stream.packet_sample_limit(packet.duration as u64).unwrap() {
                    pcm.data.truncate(limit * 4);
                }
                if let Some(pcm) = stream.present_decoded(pcm, packet.pts).unwrap() {
                    output.extend(pcm.data);
                }
            }
        }
        assert_eq!(output.len(), 48000 * 4);
        output
    }
    let control = decode(include_bytes!("fixtures/playback-errors/control-aac.mp4"));
    for fixture in [
        include_bytes!("fixtures/playback-errors/aac-quicktime-v1.mov").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-stale-channels.mp4").as_slice(),
    ] {
        assert_eq!(decode(fixture), control);
        assert_eq!(frames(fixture).unwrap().len(), 12);
    }
}

#[cfg(feature = "player")]
#[test]
fn opus_regression_now_decodes_and_presents_audio() {
    use fvid::audio::AudioStream;
    let bytes = include_bytes!("fixtures/playback-errors/opus-regression.webm");
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(bytes),
        fvid::container::webm::Limits::default(),
    )
    .unwrap();
    assert_eq!(stream.codec(), "A_OPUS");
    let mut decoder = fvid::codec::make_audio_decoder(
        stream.codec(),
        stream.extra_data(),
        stream.sample_rate(),
        stream.channels(),
        0,
    )
    .unwrap();
    let mut frames = 0;
    while let Some(packet) = stream.next_packet().unwrap() {
        let pcm = decoder
            .decode_encoded(
                &packet.data,
                packet.pts.max(0) as u64,
                packet.duration as u64,
            )
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream.present_decoded(pcm, packet.pts).unwrap() {
            frames += pcm.data.len() / 4;
        }
    }
    assert_eq!(frames, 48000);
}

#[test]
fn seeking_zero_in_a_video_with_a_delayed_first_picture_returns_that_picture() {
    let bytes = include_bytes!("fixtures/playback-errors/delayed-video-start.mp4");
    let mut reader = NativeReader::software(Cursor::new(bytes), 16 << 20).unwrap();
    assert!(reader.seek_raw(Duration::ZERO).unwrap().is_some());
    assert_eq!(reader.frame_interval(), Some((1000, 2000, 12000)));
}
