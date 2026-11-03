use std::io::Cursor;
const INPUTS: [&[u8]; 4] = [
    include_bytes!("../../../fixtures/playback-errors/mp4-uniform-seek-width/pcm-1000.mp4"),
    include_bytes!("../../../fixtures/playback-errors/mp4-uniform-seek-width/pcm-48000.mp4"),
    include_bytes!("../../../fixtures/playback-errors/mp4-uniform-seek-width/alac-1000.mp4"),
    include_bytes!("../../../fixtures/playback-errors/mp4-uniform-seek-width/alac-48000.mp4"),
];
#[unsafe(no_mangle)]
pub extern "C" fn pointer_width() -> u32 {
    usize::BITS
}
#[unsafe(no_mangle)]
pub extern "C" fn sample_at(case: u32, target: i64) -> u32 {
    let reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
        Cursor::new(INPUTS[case as usize]),
        Default::default(),
    )
    .unwrap();
    let samples = &reader.tracks()[1].samples;
    assert_eq!(samples.len(), 3);
    assert_eq!(
        matches!(samples, fvid_media::owned_mp4::SampleIndex::Uniform(_)),
        case < 2
    );
    samples.at_or_before(target) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn synthetic_at(count: u32, duration: u32, target: i64) -> u32 {
    use fvid_media::owned_mp4::{SampleIndex, UniformIndex};
    SampleIndex::Uniform(UniformIndex {
        bytes_per_frame: 2,
        duration,
        count: count as usize,
        runs: vec![],
    })
    .at_or_before(target) as u32
}
