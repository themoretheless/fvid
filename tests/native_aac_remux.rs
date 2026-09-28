use fvid::container::{adts, mp4, mp4_write};
#[test]
fn adts_to_mp4_preserves_packets_clock_and_owned_pcm() {
    for data in [include_bytes!("fixtures/audio/aac-stereo.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-mono-44k.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-51-active.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-96k.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-88k.aac").as_slice()] {
        let source = adts::Aac::parse(data, &Default::default()).unwrap();
        let mut output = Vec::new();
        assert_eq!(mp4_write::write_adts_aac(data, &mut output).unwrap(), source.packets() as u64);
        let mut reader = mp4::Mp4Reader::open(std::io::Cursor::new(&output), Default::default()).unwrap();
        let track = &reader.tracks()[0];
        assert_eq!((track.sample_rate,track.channels,track.duration), (source.sample_rate,source.channels,source.samples()));
        assert!(track.edits.is_empty());
        let mut packet = Vec::new();
        for index in 0..source.packets() {
            assert_eq!(reader.tracks()[0].samples.get(index).unwrap().pts, index as i64*1024);
            reader.read_packet(0,index,&mut packet).unwrap();
            assert_eq!(packet, source.packet(index));
        }
        let (mut before, mut after) = (Vec::new(),Vec::new());
        fvid::native_media::decode_aac_pcm(data,&mut before,&Default::default()).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&output,&mut after).unwrap();
        assert_eq!(before,after);
    }
}

#[test]
fn remux_cli_never_overwrites_and_rejects_truncated_tail() {
    let dir = std::env::temp_dir().join(format!("fvid-remux-{}",std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.aac");
    let output = dir.join("out.m4a");
    let fixture = include_bytes!("fixtures/audio/aac-stereo.aac");
    std::fs::write(&source,fixture).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media","remux"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(),"{}",String::from_utf8_lossy(&run.stderr));
    let original = std::fs::read(&output).unwrap();
    assert!(fvid::native_export::remux_adts_aac(&source,&output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(),original);
    std::fs::write(&source,&fixture[..fixture.len()-1]).unwrap();
    let failed = dir.join("failed.m4a");
    assert!(fvid::native_export::remux_adts_aac(&source,&failed).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(),2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn extended_audio_entry_rejects_invalid_rates_and_channels() {
    let mut bytes = Vec::new();
    mp4_write::write_adts_aac(include_bytes!("fixtures/audio/aac-96k.aac"), &mut bytes).unwrap();
    let entry = bytes.windows(4).rposition(|s| s == b"mp4a").unwrap()+4;
    assert_eq!(&bytes[entry+8..entry+10], &2u16.to_be_bytes());
    assert_eq!(f64::from_be_bytes(bytes[entry+32..entry+40].try_into().unwrap()),96000.0);
    for rate in [f64::NAN,f64::INFINITY,-1.0,0.0,96000.5] {
        let mut invalid = bytes.clone();
        invalid[entry+32..entry+40].copy_from_slice(&rate.to_be_bytes());
        assert!(mp4::Mp4Reader::open(std::io::Cursor::new(invalid),Default::default()).is_err());
    }
    for channels in [0u32,65536] {
        let mut invalid = bytes.clone();
        invalid[entry+40..entry+44].copy_from_slice(&channels.to_be_bytes());
        assert!(mp4::Mp4Reader::open(std::io::Cursor::new(invalid),Default::default()).is_err());
    }
}
