use fvid_cpu::{hflip_row, hflip_row_copy};

#[test]
fn invalid_copy_geometry_leaves_the_destination_unchanged() {
    for (width, step, src_len, dst_len) in [
        (4, 1, 3, 4),
        (4, 1, 4, 3),
        (3, 2, 5, 6),
        (3, 2, 6, 5),
        (usize::MAX, 2, 4, 4),
        (usize::MAX / 2 + 1, 2, 4, 4),
    ] {
        let src = vec![1; src_len];
        let mut dst = vec![0xa5; dst_len];
        hflip_row_copy(&mut dst, &src, width, step);
        assert_eq!(dst, vec![0xa5; dst_len], "{width} x {step}");
    }
}

#[test]
fn copy_matches_in_place_without_touching_row_padding() {
    for width in [0, 1, 2, 3, 31, 32, 33, 63, 64, 65, 127, 128, 129] {
        for step in [0, 1, 2, 3, 4, 6, 8] {
            let bytes = width * step;
            let src: Vec<_> = (0..bytes + 7).map(|i| (i % 251) as u8).collect();
            let mut expected = src.clone();
            hflip_row(&mut expected, width, step);
            let mut actual = vec![0xa5; bytes + 7];
            hflip_row_copy(&mut actual, &src, width, step);
            assert_eq!(&actual[..bytes], &expected[..bytes]);
            assert_eq!(&actual[bytes..], &[0xa5; 7]);
        }
    }
}
