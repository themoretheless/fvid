//! Explicit bounded robustness audit; no accepted improvement by itself.
use fvid::codec::avc_decoder::AvcDecoder;
fn main() {
    let root = std::path::Path::new("tests/fixtures/playback-errors");
    let config = std::fs::read(root.join("avc-component-depth-y8-c8.avcc")).unwrap();
    let original = std::fs::read(root.join("avc-component-depth-y8-c8-0.packet")).unwrap();
    let mut healthy = AvcDecoder::new(&config, 32 << 20).unwrap();
    assert!(
        healthy.decode(&original).unwrap().is_some(),
        "healthy original must decode"
    );
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut checked = 0usize;
    let mut refused = 0usize;
    for length in 5..=original.len() {
        for bit in 0..8 {
            let mut packet = original[..length].to_vec();
            let n = packet.len() - 4;
            packet[..4].copy_from_slice(&(n as u32).to_be_bytes());
            if length > 5 {
                packet[length - 1] ^= 1 << bit;
            }
            let result = std::panic::catch_unwind(|| {
                let mut d = AvcDecoder::new(&config, 32 << 20).unwrap();
                d.decode(&packet)
            });
            checked += 1;
            match result {
                Ok(Err(_)) => refused += 1,
                Ok(_) => {}
                Err(_) => {
                    std::fs::write("/private/tmp/fvid-avc-mutation-panic.packet", &packet).unwrap();
                    std::panic::set_hook(old);
                    panic!("panic at length {length} bit {bit}");
                }
            }
        }
    }
    // Keep the admitted SPS/PPS and NAL length/header intact. Mutate slice
    // syntax and sample payload independently; successful changed pixels are
    // ordinary valid input, not defects.
    for offset in 5..original.len() {
        for mask in [1u8, 2, 4, 8, 16, 32, 64, 128, 255] {
            let mut packet = original.clone();
            packet[offset] ^= mask;
            let result = std::panic::catch_unwind(|| {
                let mut d = AvcDecoder::new(&config, 32 << 20).unwrap();
                d.decode(&packet)
            });
            checked += 1;
            match result {
                Ok(Err(_)) => refused += 1,
                Ok(_) => {},
                Err(_) => {
                    std::fs::write("/private/tmp/fvid-avc-mutation-panic.packet", &packet).unwrap();
                    std::panic::set_hook(old);
                    panic!("panic at byte {offset} mask {mask}");
                }
            }
        }
    }
    std::panic::set_hook(old);
    println!("checked={checked} refused={refused}");
}
