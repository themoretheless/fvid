//! Owned WebM packet-to-planar adapter for the shared streaming pipeline.
use crate::owned_frame::{GeometryFrame, buffer};
use fvid_codecs::{
    Error,
    codec::{av1_decoder, vp9, vp9_decoder},
};
type Result<T> = std::result::Result<T, String>;
pub(crate) const UNSUPPORTED_PREFIX: &str = "unsupported owned video capability: ";
fn core_error(error: Error) -> String {
    match error {
        Error::Unsupported(value) => format!("{UNSUPPORTED_PREFIX}{value}"),
        other => other.to_string(),
    }
}
pub(crate) struct Decoded {
    pub frame: GeometryFrame,
    pub depth: u8,
}
enum Kind {
    Ffv1(crate::owned_ffv1_decoder::Decoder),
    Vp9(vp9_decoder::Decoder),
    Av1(av1_decoder::Decoder),
}
pub(crate) struct Decoder {
    kind: Kind,
    mono: Option<bool>,
    colour: Option<(bool, u8, u8, u8)>,
}
impl Decoder {
    pub fn new(codec: &str, private: &[u8], width: usize, height: usize) -> Result<Option<Self>> {
        let kind = match codec {
            "V_FFV1" if private.is_empty() => Kind::Ffv1(crate::owned_ffv1_decoder::Decoder::new(
                width,
                height,
                usize::MAX,
            )?),
            "V_VP9" => Kind::Vp9(vp9_decoder::Decoder::new(usize::MAX)),
            "V_AV1" => {
                let decoder = match av1_decoder::Decoder::from_configuration(private, usize::MAX) {
                    Ok(value) => value,
                    Err(Error::Unsupported(_)) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                };
                Kind::Av1(decoder)
            }
            _ => return Ok(None),
        };
        Ok(Some(Self {
            kind,
            mono: None,
            colour: None,
        }))
    }
    pub fn monochrome(&self) -> Option<bool> {
        self.mono
    }
    pub fn colour(&self) -> Option<(bool, u8, u8, u8)> {
        self.colour
    }
    pub fn hdr(&self) -> Option<crate::owned_matroska::HdrMetadata> {
        let Kind::Av1(decoder) = &self.kind else {
            return None;
        };
        let hdr = decoder.hdr();
        let corner = |value: fvid_codecs::color::primaries::Chromaticity| {
            crate::owned_matroska::Chromaticity {
                x: value.x,
                y: value.y,
            }
        };
        Some(crate::owned_matroska::HdrMetadata {
            mastering: hdr
                .mastering
                .map(|m| crate::owned_matroska::MasteringDisplay {
                    red: corner(m.red),
                    green: corner(m.green),
                    blue: corner(m.blue),
                    white: corner(m.white),
                    max_luminance: m.max_luminance,
                    min_luminance: m.min_luminance,
                }),
            light: crate::owned_matroska::ContentLight {
                max_cll: hdr.light.max_cll,
                max_fall: hdr.light.max_fall,
            },
        })
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Decoded>> {
        let mut shown = None;
        match &mut self.kind {
            Kind::Ffv1(decoder) => {
                let decoded = decoder.decode(packet)?;
                self.mono = decoder.monochrome();
                return Ok(Some(Decoded {
                    frame: decoded.frame,
                    depth: decoded.depth,
                }));
            }
            Kind::Vp9(decoder) => {
                for frame in vp9::frames(packet).map_err(core_error)? {
                    let decoded = decoder.decode(frame).map_err(core_error)?;
                    if !decoded.header.show_frame {
                        continue;
                    }
                    if shown.is_some() {
                        return Err(
                            "multiple visible frames in one WebM packet need distinct timestamps"
                                .into(),
                        );
                    }
                    let format = decoded.header.picture.format;
                    if format.color_space == 7 {
                        return Err(format!("{UNSUPPORTED_PREFIX}RGB VP9 streaming transforms"));
                    }
                    let matrix = match format.color_space {
                        2 => 1,
                        4 => 7,
                        5 => 9,
                        _ => 6,
                    };
                    self.colour = Some((format.full_range, matrix, 0, 0));
                    self.mono = Some(false);
                    let picture = &decoded.picture;
                    let planes = picture
                        .planes
                        .each_ref()
                        .map(|plane| (plane.width, plane.height, plane.samples.as_slice()));
                    shown = Some(pack(
                        picture.size,
                        picture.depth,
                        planes,
                        format.subsampling,
                        false,
                    )?);
                }
            }
            Kind::Av1(decoder) => {
                for decoded in decoder.decode_packet(packet).map_err(core_error)? {
                    if !decoded.show {
                        continue;
                    }
                    if shown.is_some() {
                        return Err(
                            "multiple visible frames in one WebM packet need distinct timestamps"
                                .into(),
                        );
                    }
                    if decoded.color.matrix == 0 && !decoded.color.monochrome {
                        return Err(format!("{UNSUPPORTED_PREFIX}RGB AV1 streaming transforms"));
                    }
                    if decoded.color.monochrome && !decoded.color.full_range {
                        return Err(format!(
                            "{UNSUPPORTED_PREFIX}studio-range monochrome streaming transforms"
                        ));
                    }
                    self.colour = Some((
                        decoded.color.full_range,
                        decoded.color.matrix,
                        decoded.color.primaries,
                        decoded.color.transfer,
                    ));
                    self.mono = Some(decoded.color.monochrome);
                    let picture = &decoded.picture;
                    let planes = picture
                        .planes
                        .each_ref()
                        .map(|plane| (plane.width, plane.height, plane.samples.as_slice()));
                    shown = Some(pack(
                        picture.size,
                        picture.depth,
                        planes,
                        decoded.color.subsampling,
                        decoded.color.monochrome,
                    )?);
                }
            }
        }
        Ok(shown)
    }
}
fn pack(
    size: [u32; 2],
    depth: u8,
    planes: [(usize, usize, &[u16]); 3],
    sub: [bool; 2],
    mono: bool,
) -> Result<Decoded> {
    let [width, height] = size.map(|v| v as usize);
    let subsampling = if mono {
        [1, 1]
    } else {
        sub.map(|v| if v { 2 } else { 1 })
    };
    let samples = width
        .checked_mul(height)
        .ok_or("video plane size overflow")?;
    let chroma = width
        .div_ceil(subsampling[0])
        .checked_mul(height.div_ceil(subsampling[1]))
        .ok_or("video plane size overflow")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let mut data = buffer(
        samples
            .checked_add(chroma.checked_mul(2).ok_or("video frame size overflow")?)
            .and_then(|v| v.checked_mul(bytes))
            .ok_or("video frame size overflow")?,
    )?;
    let mut cursor = 0;
    for (component, &(pw, ph, values)) in planes.iter().enumerate() {
        let count = if component == 0 { samples } else { chroma };
        let w = if component == 0 {
            width
        } else {
            width.div_ceil(subsampling[0])
        };
        let h = if component == 0 {
            height
        } else {
            height.div_ceil(subsampling[1])
        };
        if !(mono && component != 0)
            && (pw < w || ph < h || pw.checked_mul(ph) != Some(values.len()))
        {
            return Err("decoded video plane geometry mismatch".into());
        }
        debug_assert_eq!(w * h, count);
        for row in 0..h {
            for col in 0..w {
                let value = if mono && component != 0 {
                    1u16 << (depth - 1)
                } else {
                    values[row * pw + col]
                };
                if bytes == 1 {
                    data[cursor] =
                        u8::try_from(value).map_err(|_| "8-bit video sample overflow")?;
                    cursor += 1;
                } else {
                    data[cursor..cursor + 2].copy_from_slice(&value.to_le_bytes());
                    cursor += 2;
                }
            }
        }
    }
    Ok(Decoded {
        frame: GeometryFrame {
            width,
            height,
            subsampling: Some(subsampling),
            data,
        },
        depth,
    })
}
