// Independent RIFF chunk walker for exact PCM assertions in CLI regressions.
pub fn data(wav: &[u8]) -> &[u8] {
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    let mut at = 12;
    let mut data = None;
    while at < wav.len() {
        assert!(at + 8 <= wav.len(), "truncated WAVE chunk header");
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = at.checked_add(8).unwrap().checked_add(size).unwrap();
        assert!(end <= wav.len(), "truncated WAVE chunk body");
        if &wav[at..at + 4] == b"data" {
            assert!(data.is_none(), "duplicate WAVE data chunk");
            data = Some(&wav[at + 8..end]);
        }
        at = end + (size & 1);
    }
    data.expect("missing WAVE data chunk")
}
