use std::{path::PathBuf, time::Duration};
fn atom(id: u32, data: &[u8]) -> Vec<u8> {
    let id = id.to_be_bytes();
    let mut out = id[id.iter().position(|n| *n != 0).unwrap()..].to_vec();
    let n = data.len() as u64;
    for width in 1..=8 {
        if n < (1u64 << (7 * width)) - 1 {
            let size = (n | (1u64 << (7 * width))).to_be_bytes();
            out.extend_from_slice(&size[8 - width..]);
            break;
        }
    }
    out.extend_from_slice(data);
    out
}
fn uint(id: u32, value: u64) -> Vec<u8> {
    let data = value.to_be_bytes();
    atom(id, &data[data.iter().position(|n| *n != 0).unwrap_or(7)..])
}
fn source(packets: &[(u64, [i16; 4], i64)]) -> Vec<u8> {
    let audio = [
        atom(0xb5, &8000f64.to_be_bytes()),
        uint(0x9f, 1),
        uint(0x6264, 16),
    ]
    .concat();
    let track = [
        uint(0xd7, 1),
        uint(0x83, 2),
        atom(0x86, b"A_PCM/INT/LIT"),
        atom(0xe1, &audio),
    ]
    .concat();
    let mut segment = [
        atom(0x1549a966, &uint(0x2ad7b1, 1)),
        atom(0x1654ae6b, &atom(0xae, &track)),
    ]
    .concat();
    for (pts, samples, padding) in packets {
        let block = [
            vec![0x81, 0, 0, 0],
            samples
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect(),
        ]
        .concat();
        let group = [
            atom(0xa1, &block),
            uint(0x9b, 500_000),
            atom(0x75a2, &padding.to_be_bytes()),
        ]
        .concat();
        segment.extend(atom(
            0x1f43b675,
            &[uint(0xe7, *pts), atom(0xa0, &group)].concat(),
        ));
    }
    [
        atom(0x1a45dfa3, &atom(0x4282, b"matroska")),
        atom(0x18538067, &segment),
    ]
    .concat()
}
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path =
        std::env::temp_dir().join(format!("fvid-audio-timeline-{name}-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    Dir(path)
}
fn render(
    name: &str,
    packets: &[(u64, [i16; 4], i64)],
    interval: Option<(Duration, Duration)>,
) -> Vec<f32> {
    let d = dir(name);
    let input = d.0.join("source.mka");
    let output = d.0.join("out.wav");
    std::fs::write(&input, source(packets)).unwrap();
    let stats = fvid::native_export::export_audio_pcm_selected(
        &input, &output, interval, 1.0, None, None, None, None, None,
    )
    .unwrap();
    let bytes = std::fs::read(output).unwrap();
    let mut at = 12;
    while &bytes[at..at + 4] != b"data" {
        let n = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        at += 8 + n + n % 2;
    }
    let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    let samples = bytes[at + 8..at + 8 + size]
        .chunks_exact(4)
        .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(stats.sample_frames, samples.len() as u64);

    let mut precise = Vec::new();
    let own = fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
        std::io::Cursor::new(std::fs::read(&input).unwrap()),
        &mut precise,
        interval,
        &Default::default(),
    )
    .unwrap();
    let expected: Vec<u8> = samples
        .iter()
        .flat_map(|s| f64::from(*s).to_le_bytes())
        .collect();
    assert_eq!(precise, expected);
    assert_eq!(own.sample_frames, stats.sample_frames);
    let library_output = d.0.join("owned.wav");
    fvid_media::owned_audio_export::decode_audio_transformed(
        &input,
        &library_output,
        fvid_media_info::AudioDecodeTransform {
            interval: interval.map(|(from, to)| (from.as_micros() as i64, to.as_micros() as i64)),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let wave = std::fs::read(library_output).unwrap();
    let info =
        fvid_media::owned_wave_inspect::inspect(&mut std::io::Cursor::new(&wave), None).unwrap();
    assert_eq!(info.bits_per_sample, 64);
    assert_eq!(
        &wave[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
        expected
    );

    samples
}
fn values(values: &[i16]) -> Vec<f32> {
    values.iter().map(|n| f32::from(*n) / 32768.0).collect()
}
#[test]
fn padding_makes_overlapping_coded_intervals_contiguous_in_presentation() {
    let actual = render(
        "trim",
        &[
            (0, [1, 2, 3, 4], 0),
            (250_000, [99, 99, 5, 6], -250_000),
            (750_000, [7, 8, 99, 99], 250_000),
            (1_000_000, [9, 10, 11, 12], 0),
        ],
        None,
    );
    assert_eq!(actual, values(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));
}
#[test]
fn presentation_gaps_are_silence_and_interval_selection_uses_that_clock() {
    let packets = [(0, [1, 2, 3, 4], 0), (1_000_000, [5, 6, 7, 8], 0)];
    assert_eq!(
        render("gap", &packets, None),
        values(&[1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8])
    );
    assert_eq!(
        render(
            "interval",
            &packets,
            Some((Duration::from_micros(625), Duration::from_micros(1375)))
        ),
        values(&[0, 0, 0, 5, 6, 7])
    );
}
#[test]
fn untrimmed_overlap_fails_without_publication() {
    let d = dir("overlap");
    let input = d.0.join("source.mka");
    let output = d.0.join("out.wav");
    std::fs::write(
        &input,
        source(&[(0, [1, 2, 3, 4], 0), (250_000, [5, 6, 7, 8], 0)]),
    )
    .unwrap();
    let result = fvid::native_export::export_audio_pcm_selected(
        &input, &output, None, 1.0, None, None, None, None, None,
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("overlapping presented"));
    assert!(!output.exists());

    let owned_output = d.0.join("owned-overlap.wav");
    assert!(fvid_media::owned_audio_export::decode_audio(
        &input,
        &owned_output,
        &Default::default()
    )
    .unwrap_err()
    .contains("overlapping presented"));
    assert!(!owned_output.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
