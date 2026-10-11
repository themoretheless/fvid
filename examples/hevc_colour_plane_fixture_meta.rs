//! Offline metadata helper for the authored HEVC fixture generators.
use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("missing synthetic fixture path")?;
    let mut r = Mp4Reader::open(std::fs::File::open(path)?, Default::default())?;
    let config = r.tracks()[0].configuration.clone();
    let d = HevcDecoder::from_configuration(&config, 16 << 20)?;
    let (s, p) = d.parameters();
    let mut packet = vec![];
    let mut packets = vec![];
    for i in 0..r.tracks()[0].samples.len() {
        r.read_packet(0, i, &mut packet)?;
        let mut slices = vec![];
        for header in d.slice_headers(&packet)? {
            slices.push(serde_json::json!({"short_term_bit_length":header.short_term_bit_length,"entropy_byte_offset":header.entropy_byte_offset,"rbsp":header.rbsp,"idr":header.nal.is_irap(),"first":header.first,"dependent":header.dependent,"kind":header.nal.unit_type,"temporal_mvp":header.temporal_mvp,"long_term":header.long_term.len()}));
        }
        packets.push(slices);
    }
    println!(
        "{}",
        serde_json::json!({"configuration":config,"dimensions":s.dimensions,"chroma_format":s.chroma_format,"separate_colour_plane":s.separate_colour_plane,"poc_bits":s.poc_bits,"extra_bits":p.extra_slice_header_bits,"output_flag":p.output_flag_present,"packets":packets,"depth":s.depth[0],"chroma_depth":s.depth[1],"pcm_depth":s.pcm.as_ref().map(|pcm| pcm.depth[0]),"dependent_enabled":p.dependent_slices,"short_term_sets":s.short_term.len(),"long_term_entries":s.long_term.len(),"long_term_present":s.long_term_present,"temporal_mvp_enabled":s.temporal_mvp,"ctu_count":s.dimensions.map(|v| v.div_ceil(1 << s.coding_block_log2[1])).iter().product::<u32>()})
    );
    Ok(())
}
