#[cfg(feature = "player")]
pub fn play(
    source: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    fn present(
        source: &mut dyn fvid::audio::AudioStream,
        f: fvid::audio::DecodedAudio,
        out: &mut Vec<u8>,
    ) {
        if let Some(p) = source.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    let mut out = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => {
                panic!("constant grouped transport unexpectedly changed config")
            }
            AudioStep::Pcm(p) => out.extend(p.data),
            AudioStep::Encoded(p) => {
                let stamp = source.decoder_stamp(&p);
                if let Some(f) = source
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    present(source, f, &mut out);
                }
            }
        }
    }
    if source.drain_decoded_at_eof() {
        while let Some(f) = decoder.finish_packet().unwrap() {
            present(source, f, &mut out);
        }
    }
    source.validate_eof().unwrap();
    out
}
