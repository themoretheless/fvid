use fvid::{
    codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::HevcDecoder,
        hevc_slice::SliceHeader,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;

#[test]
fn separate_colour_plane_reproducer_reaches_valid_plane_headers() {
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-separate-colour-planes-pcm-synthetic.mp4");
    let mut reader = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(reader.tracks()[0].samples.len(), 1);
    let config = reader.tracks()[0].configuration.clone();
    let h = HevcConfig::parse(&config).unwrap();
    let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let (sps, pps) = decoder.parameters();
    assert!(sps.separate_colour_plane);
    assert_eq!(sps.chroma_format, 3);
    let mut packet = vec![];
    reader.read_packet(0, 0, &mut packet).unwrap();
    let headers: Vec<_> = NalUnits::new(&packet, h.length_size)
        .unwrap()
        .map(|n| SliceHeader::parse(n.unwrap(), sps, pps, 16 << 20).unwrap())
        .collect();
    assert_eq!(
        headers.iter().map(|h| h.colour_plane).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(headers.iter().all(|h| h.first && h.entropy_byte_offset > 0));
    // Each plane's entropy stream must remain decodable as the authored mono
    // seed. This rules out a damaged CABAC/PCM payload as the failure cause.
    let mut mono = sps.clone();
    mono.chroma_format = 0;
    mono.separate_colour_plane = false;
    let gold = include_bytes!("fixtures/playback-errors/hevc-pcm-mono-active-rext8.yuv");
    for header in &headers {
        let picture =
            fvid::codec::hevc_picture::decode(&mono, pps, header, 0, &[vec![], vec![]], 16 << 20)
                .unwrap();
        assert_eq!(
            picture.planes[0]
                .samples()
                .iter()
                .map(|&v| v as u8)
                .collect::<Vec<_>>(),
            gold
        );
    }
    // Reproduction/refusal only; this is not playback acceptance.
    assert_eq!(
        decoder.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC slice addresses must increase within one picture"
    );
}
