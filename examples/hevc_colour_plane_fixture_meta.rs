//! Offline metadata helper for the authored separate-colour-plane fixture generator.
use fvid::{
    codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::HevcDecoder,
        hevc_slice::SliceHeader,
    },
    container::mp4::Mp4Reader,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("missing synthetic fixture path")?;
    let mut r = Mp4Reader::open(std::fs::File::open(path)?, Default::default())?;
    let config = r.tracks()[0].configuration.clone();
    let h = HevcConfig::parse(&config)?;
    let d = HevcDecoder::from_configuration(&config, 16 << 20)?;
    let (s, p) = d.parameters();
    let mut packet = vec![];
    r.read_packet(0, 0, &mut packet)?;
    let mut slices = vec![];
    for nal in NalUnits::new(&packet, h.length_size)? {
        let nal = nal?;
        if (nal[0] >> 1) & 63 > 31 {
            continue;
        }
        let header = SliceHeader::parse(nal, s, p, 16 << 20)?;
        slices.push(serde_json::json!({"entropy_byte_offset":header.entropy_byte_offset,"rbsp":header.rbsp,"idr":header.nal.is_idr()}));
    }
    println!(
        "{}",
        serde_json::json!({"extra_bits":p.extra_slice_header_bits,"output_flag":p.output_flag_present,"slices":slices})
    );
    Ok(())
}
