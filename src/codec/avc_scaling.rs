//! H.264 scaling-list fallback rules A/B and progressive raster weights.
//! Default matrices are normative H.264 tables 7-3/7-4; no decoder dependency.
use super::{
    avc::{Pps, ScalingList, Sps},
    avc_transform::FRAME_SCAN_4X4,
    avc_transform8::FRAME_SCAN,
};
use crate::{Result, invalid};
const DEFAULT4: [[u8; 16]; 2] = [
    [
        6, 13, 20, 28, 13, 20, 28, 32, 20, 28, 32, 37, 28, 32, 37, 42,
    ],
    [
        10, 14, 20, 24, 14, 20, 24, 27, 20, 24, 27, 30, 24, 27, 30, 34,
    ],
];
const DEFAULT8: [[u8; 64]; 2] = [
    [
        6, 10, 13, 16, 18, 23, 25, 27, 10, 11, 16, 18, 23, 25, 27, 29, 13, 16, 18, 23, 25, 27, 29,
        31, 16, 18, 23, 25, 27, 29, 31, 33, 18, 23, 25, 27, 29, 31, 33, 36, 23, 25, 27, 29, 31, 33,
        36, 38, 25, 27, 29, 31, 33, 36, 38, 40, 27, 29, 31, 33, 36, 38, 40, 42,
    ],
    [
        9, 13, 15, 17, 19, 21, 22, 24, 13, 13, 17, 19, 21, 22, 24, 25, 15, 17, 19, 21, 22, 24, 25,
        27, 17, 19, 21, 22, 24, 25, 27, 28, 19, 21, 22, 24, 25, 27, 28, 30, 21, 22, 24, 25, 27, 28,
        30, 32, 22, 24, 25, 27, 28, 30, 32, 33, 24, 25, 27, 28, 30, 32, 33, 35,
    ],
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScalingMatrices {
    /// Intra Y/Cb/Cr, then inter Y/Cb/Cr, in raster order.
    pub four: [[u8; 16]; 6],
    /// Intra then inter luma, in raster order.
    pub eight: [[u8; 64]; 2],
}
fn resolve<const N: usize>(
    entry: Option<&ScalingList>,
    fallback: [u8; N],
    default: [u8; N],
    scan: &[usize; N],
) -> Result<[u8; N]> {
    match entry {
        None => Ok(fallback),
        Some(ScalingList::Default) => Ok(default),
        Some(ScalingList::Explicit(values)) => {
            if values.len() != N || values.contains(&0) {
                return Err(invalid("invalid AVC scaling-list weights"));
            }
            let mut raster = [0; N];
            for (i, &position) in scan.iter().enumerate() {
                raster[position] = values[i];
            }
            Ok(raster)
        }
    }
}
impl ScalingMatrices {
    /// H.264 8.5: separate planes use their own 4x4 and 8x8 scaling lists.
    /// Resolve fallback before projecting the selected component onto luma.
    pub fn for_colour_plane(sps: &Sps, pps: &Pps, plane: usize) -> Result<Self> {
        if sps.chroma_format != 3 || !sps.separate_colour_plane || plane >= 3 {
            return Err(invalid("invalid AVC separate scaling plane"));
        }
        if sps
            .scaling_lists
            .as_ref()
            .is_some_and(|lists| lists.len() != 12)
            || pps
                .scaling_lists
                .as_ref()
                .is_some_and(|lists| !matches!(lists.len(), 6 | 12))
        {
            return Err(invalid("invalid AVC separate-plane scaling-list count"));
        }
        let mut mono = sps.clone();
        mono.chroma_format = 0;
        mono.separate_colour_plane = false;
        if let Some(lists) = &mut mono.scaling_lists {
            lists.truncate(8);
        }
        let mut picture = pps.clone();
        if let Some(lists) = &mut picture.scaling_lists {
            lists.truncate(8);
        }
        let mut result = Self::new(&mono, &picture)?;
        result.four[0] = result.four[plane];
        result.four[3] = result.four[3 + plane];
        let mut eight = [[16; 64]; 6];
        if let Some(lists) = &sps.scaling_lists {
            for i in 0..6 {
                let fallback = if i < 2 { DEFAULT8[i] } else { eight[i - 2] };
                eight[i] = resolve(
                    lists[6 + i].as_ref(),
                    fallback,
                    DEFAULT8[i % 2],
                    &FRAME_SCAN,
                )?;
            }
        }
        if let Some(lists) = &pps.scaling_lists {
            if lists.len() > 6 {
                let sequence = eight;
                for i in 0..6 {
                    let fallback = if i >= 2 {
                        eight[i - 2]
                    } else if sps.scaling_lists.is_some() {
                        sequence[i]
                    } else {
                        DEFAULT8[i]
                    };
                    eight[i] = resolve(
                        lists.get(6 + i).and_then(Option::as_ref),
                        fallback,
                        DEFAULT8[i % 2],
                        &FRAME_SCAN,
                    )?;
                }
            }
        }
        result.eight = [eight[plane * 2], eight[plane * 2 + 1]];
        Ok(result)
    }

    pub fn new(sps: &Sps, pps: &Pps) -> Result<Self> {
        if sps.chroma_format > 2 {
            return Err(invalid("AVC scaling reconstruction requires 4:2:0"));
        }
        let mut result = Self {
            four: [[16; 16]; 6],
            eight: [[16; 64]; 2],
        };
        if let Some(lists) = &sps.scaling_lists {
            if lists.len() != 8 {
                return Err(invalid("invalid AVC SPS scaling-list count"));
            }
            for i in 0..6 {
                let fallback = if i == 0 || i == 3 {
                    DEFAULT4[i / 3]
                } else {
                    result.four[i - 1]
                };
                result.four[i] = resolve(
                    lists[i].as_ref(),
                    fallback,
                    DEFAULT4[i / 3],
                    &FRAME_SCAN_4X4,
                )?;
            }
            for i in 0..2 {
                result.eight[i] =
                    resolve(lists[6 + i].as_ref(), DEFAULT8[i], DEFAULT8[i], &FRAME_SCAN)?;
            }
        }
        if let Some(lists) = &pps.scaling_lists {
            if lists.len() != 6 && lists.len() != 8 {
                return Err(invalid("invalid AVC PPS scaling-list count"));
            }
            let sequence = result.clone();
            for i in 0..6 {
                let fallback = if i == 0 || i == 3 {
                    if sps.scaling_lists.is_some() {
                        sequence.four[i]
                    } else {
                        DEFAULT4[i / 3]
                    }
                } else {
                    result.four[i - 1]
                };
                result.four[i] = resolve(
                    lists[i].as_ref(),
                    fallback,
                    DEFAULT4[i / 3],
                    &FRAME_SCAN_4X4,
                )?;
            }
            if lists.len() > 6 {
                for i in 0..2 {
                    let fallback = if sps.scaling_lists.is_some() {
                        sequence.eight[i]
                    } else {
                        DEFAULT8[i]
                    };
                    result.eight[i] = resolve(
                        lists.get(6 + i).and_then(Option::as_ref),
                        fallback,
                        DEFAULT8[i],
                        &FRAME_SCAN,
                    )?;
                }
            }
        }
        Ok(result)
    }
}
