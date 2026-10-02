//! Standard speaker masks used by owned PCM rematrixing.
/// FL/FR/FC, quad with BC, 5.0/5.1 back, 6.1 and 7.1.
pub fn standard_mask(channels: u16) -> Option<u64> {
    match channels {
        1 => Some(0x4),
        2 => Some(0x3),
        3 => Some(0x7),
        4 => Some(0x107),
        5 => Some(0x37),
        6 => Some(0x3f),
        7 => Some(0x70f),
        8 => Some(0x63f),
        _ => None,
    }
}

/// Conventional default output layout, distinct from explicit rematrix layouts.
/// In particular three unspecified output channels default to FL/FR/LFE (2.1),
/// while `standard_mask(3)` intentionally describes FL/FR/FC (3.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultLayout {
    pub channels: u16,
    pub mask: u64,
    pub name: &'static str,
}
pub fn default_layout(channels: u16) -> Option<DefaultLayout> {
    let (mask, name) = match channels {
        1 => (0x4, "mono"),
        2 => (0x3, "stereo"),
        3 => (0xb, "2.1"),
        4 => (0x107, "4.0"),
        5 => (0x37, "5.0"),
        6 => (0x3f, "5.1"),
        7 => (0x70f, "6.1"),
        8 => (0x63f, "7.1"),
        10 => (0x2d60f, "5.1.4"),
        12 => (0x2d63f, "7.1.4"),
        14 => (0x2d6ff, "9.1.4"),
        16 => (0x300002d6ff, "9.1.6"),
        24 => (0x1f80003ffff, "22.2"),
        _ => return None,
    };
    Some(DefaultLayout {
        channels,
        mask,
        name,
    })
}

/// Describe a default layout without assigning speakers to unknown channel counts.
pub fn default_layout_name(channels: i32) -> String {
    match u16::try_from(channels).ok().and_then(default_layout) {
        Some(layout) => layout.name.into(),
        None => format!("{channels} channels"),
    }
}

/// Compare speaker assignments; an unspecified layout is distinct from a mask.
pub fn layouts_equal(
    a_channels: i32,
    a_mask: Option<u64>,
    b_channels: i32,
    b_mask: Option<u64>,
) -> bool {
    a_channels == b_channels && a_mask == b_mask
}

/// Speaker identifier at a position in an ascending speaker mask.
pub fn mask_channel_at(mut mask: u64, index: u32) -> Option<i32> {
    for _ in 0..index {
        if mask == 0 {
            return None;
        }
        mask &= mask - 1;
    }
    (mask != 0).then(|| mask.trailing_zeros() as i32)
}

/// Compare ordered identifiers without allocating a temporary channel map.
pub fn ordered_channels_equal(
    channels: i32,
    mut a: impl FnMut(u32) -> Option<i32>,
    mut b: impl FnMut(u32) -> Option<i32>,
) -> bool {
    (0..channels.max(0) as u32).all(|index| a(index) == b(index))
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn custom_order_matches_speakers_but_reordering_remains_distinct() {
        let native = |index| mask_channel_at(0xb, index);
        let custom = [0, 1, 3];
        assert!(ordered_channels_equal(3, native, |i| custom
            .get(i as usize)
            .copied()));
        let reordered = [1, 0, 3];
        assert!(!ordered_channels_equal(3, native, |i| reordered
            .get(i as usize)
            .copied()));
        assert_eq!(mask_channel_at(1 << 63, 0), Some(63));
        assert_eq!(mask_channel_at(1 << 63, 1), None);
        assert_eq!(mask_channel_at(0, 0), None);
        let ambisonic = [0x400, 0x401, 0x402, 0x403, 0, 1];
        assert!(ordered_channels_equal(
            6,
            |i| ambisonic.get(i as usize).copied(),
            |i| {
                if i < 4 {
                    Some(0x400 + i as i32)
                } else {
                    mask_channel_at(3, i - 4)
                }
            }
        ));
    }
    #[test]
    fn layout_identity_preserves_speaker_assignment_and_unspecified_order() {
        assert!(layouts_equal(2, Some(3), 2, Some(3)));
        assert!(!layouts_equal(3, Some(7), 3, Some(11)));
        assert!(!layouts_equal(2, None, 2, Some(3)));
        assert!(!layouts_equal(2, None, 3, None));
        assert!(layouts_equal(2, None, 2, None));
    }
    #[test]
    fn unknown_counts_remain_unspecified_in_descriptions() {
        for channels in [i32::MIN, -1, 0, 9, 11, 65536, i32::MAX] {
            assert_eq!(
                default_layout_name(channels),
                format!("{channels} channels")
            );
        }
        assert_eq!(default_layout_name(3), "2.1");
        assert_eq!(default_layout_name(16), "9.1.6");
    }
    #[test]
    fn unspecified_three_channels_retain_lfe_without_changing_explicit_surround() {
        let layout = default_layout(3).unwrap();
        assert_eq!(layout.mask, (1 << 0) | (1 << 1) | (1 << 3));
        assert_eq!(layout.name, "2.1");
        assert_eq!(standard_mask(3), Some((1 << 0) | (1 << 1) | (1 << 2)));
        for channels in [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 24] {
            let layout = default_layout(channels).unwrap();
            assert_eq!(layout.mask.count_ones(), u32::from(layout.channels));
        }
        assert!(default_layout(0).is_none());
        assert!(default_layout(9).is_none());
    }
}
