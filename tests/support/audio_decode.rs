//! Test callers must drain delayed codecs rather than assume immediate output.
#![allow(dead_code)]
use fvid::audio::{AudioDecode, AudioPacket, AudioStream};
pub trait DecodeComplete: AudioDecode {
    /// A complete elementary stream passed in one input access unit.
    fn decode_complete(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> fvid::Result<Option<AudioPacket>> {
        let mut output = self.decode_encoded(data, pts, duration)?;
        while let Some(frame) = self.finish_packet()? {
            if let Some(packet) = output.as_mut() {
                packet.data.extend(frame.packet.data);
            } else {
                output = Some(frame.packet);
            }
        }
        Ok(output)
    }
    fn drain_presented(&mut self, stream: &mut dyn AudioStream) -> Vec<AudioPacket> {
        let mut out = Vec::new();
        while let Some(frame) = self.finish_packet().unwrap() {
            if let Some(packet) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.push(packet);
            }
        }
        out
    }
}
impl<T: AudioDecode + ?Sized> DecodeComplete for T {}
