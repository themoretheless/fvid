//! Owned FFV1 version 1 range-coded, all-intra YCbCr encoder.
//! Bitstream semantics: RFC 9043. One slice and a single residual context
//! per luma/chroma model; quantization here selects contexts, not sample loss.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_ffv1_encoder_impl.rs");

/// Stream supported Y4M transforms into independently decodable FFV1 packets.
/// The sink receives output geometry, packet bytes, source PTS and duration. It
/// owns muxing/publication; an error must discard any partially written output.
pub fn encode_y4m(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<fvid_media_info::DecodeStats> {
    encode_y4m_limited(source, transform, None, sink)
}
pub(crate) fn encode_y4m_limited(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    max_packets: Option<u64>,
    sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<fvid_media_info::DecodeStats> {
    encode_y4m_counted(source, transform, max_packets, sink).map(|(stats, _)| stats)
}
pub(crate) fn encode_y4m_counted(
    source: impl std::io::BufRead,
    transform: &fvid_media_info::DecodeTransform,
    max_packets: Option<u64>,
    mut sink: impl FnMut(&crate::owned_y4m::Header, &[u8], u64, u64) -> Result<()>,
) -> Result<(fvid_media_info::DecodeStats, u64)> {
    crate::owned_y4m_decode::visit_reader_transformed_counted(
        source,
        transform,
        max_packets,
        &mut |header, data, pts, duration| {
            let (sx, sy) = header.format.subsampling();
            let frame = GeometryFrame {
                data: data.to_vec(),
                width: header.width,
                height: header.height,
                subsampling: Some([sx, sy]),
            };
            let packet = encode(&frame, header.depth())?;
            sink(header, &packet, pts, duration)
        },
    )
}
