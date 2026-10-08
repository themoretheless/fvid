use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
#[test]
fn original_video_with_he_aac_audio_demuxes_and_decodes_the_intended_sbr_failure() {
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/he-aac-sbr-packets.json"
    ))
    .unwrap();
    let mut reader =
        Mp4Reader::open(std::io::Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
    assert_eq!(reader.tracks().len(), 2);
    assert!(reader.refused().is_empty());
    assert!(
        reader
            .tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && t.codec == *b"avc1" && !t.samples.is_empty())
    );
    let index = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let audio = &reader.tracks()[index];
    assert_eq!(
        (audio.sample_rate, audio.channels, audio.samples.len()),
        (48000, 1, 3)
    );
    let asc = config::aac_specific_config(&audio.configuration).unwrap();
    let parsed = config::AudioSpecificConfig::parse(asc).unwrap();
    assert_eq!(
        (
            parsed.core.sample_rate,
            parsed.output_sample_rate(),
            parsed.sbr_present
        ),
        (24000, 48000, Some(true))
    );
    // The compatibility LC metadata API intentionally remains a refusal; the
    // extension-aware native decoder is the acceptance engine for this fix.
    assert!(config::AacConfig::parse(asc).is_err());
    let mut adapter =
        fvid::codec::aac_decoder::AacDecoder::new(&audio.configuration, 48000, 1).unwrap();
    assert_eq!(adapter.spec().sample_rate, 48000);
    let mut decoder = NativeAacDecoder::new(asc).unwrap();
    let mut output = Vec::new();
    let mut packet = Vec::new();
    for i in 0..3 {
        reader.read_packet(index, i, &mut packet).unwrap();
        let decoded = decoder.decode(&packet).unwrap();
        let adapted = adapter
            .decode(&packet, i as u64 * 2048, 2048)
            .unwrap()
            .unwrap();
        assert_eq!(
            (adapted.pts, adapted.timebase_num, adapted.timebase_den),
            (i as u64 * 2048, 1, 48000)
        );
        assert_eq!(
            adapted.data,
            decoded
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<_>>()
        );
        output.extend(decoded);
    }
    assert_eq!(output.len(), 6144);
    assert!(output.iter().any(|v| v.abs() > 1e-5));
    let raw = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
    let offset = manifest["video"]["pcm_offset"].as_u64().unwrap() as usize;
    for (&value, b) in output
        .iter()
        .zip(raw[offset..offset + output.len() * 8].chunks_exact(8))
    {
        assert_eq!(
            value.to_bits(),
            (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_bits()
        );
    }
}
