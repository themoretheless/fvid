use fvid::{Crop, Header, Plan, Transform, process};
use std::io::Cursor;
fn input() -> Vec<u8> {
    let mut x = b"YUV4MPEG2 W4 H4 F30:1 Ip C420jpeg\nFRAME\n".to_vec();
    x.extend(0..24);
    x
}
#[test]
fn exact_crop_and_both_reflections() {
    let mut out = Vec::new();
    let stats = process(
        Cursor::new(input()),
        &mut out,
        Transform {
            crop: Some(Crop {
                x: 2,
                y: 0,
                width: 2,
                height: 4,
            }),
            horizontal: true,
            vertical: true,
        },
        1024,
    )
    .unwrap();
    assert_eq!(stats.frames, 1);
    assert!(out.starts_with(b"YUV4MPEG2 W2 H4 F30:1 Ip C420jpeg\nFRAME\n"));
    assert_eq!(
        &out[out.len() - 12..],
        &[15, 14, 11, 10, 7, 6, 3, 2, 19, 17, 23, 21]
    );
}
#[test]
fn identity_preserves_payload_and_tags() {
    let src = input();
    let mut out = Vec::new();
    process(Cursor::new(&src), &mut out, Transform::default(), 1024).unwrap();
    assert_eq!(src, out);
}
#[test]
fn rejects_invalid_inputs_without_panicking() {
    for src in [
        b"".as_slice(),
        b"YUV4MPEG2 W0 H4\n",
        b"YUV4MPEG2 W4 H4 C420p10\n",
        b"YUV4MPEG2 W4 H4 It\n",
        b"YUV4MPEG2 W4 W4 H4\n",
        b"YUV4MPEG2 W4 H4\nFRAME\n\x00",
        b"YUV4MPEG2 W4 H4\nBAD\n",
        "YUV4MPEG2 W4 H4 é\n".as_bytes(),
    ] {
        assert!(process(Cursor::new(src), Vec::new(), Transform::default(), 1024).is_err());
    }
    assert!(
        process(
            Cursor::new(vec![b'x'; 8192]),
            Vec::new(),
            Transform::default(),
            1024
        )
        .is_err()
    );
}
#[test]
fn budgets_and_checked_crop() {
    let h = Header::parse(b"YUV4MPEG2 W4 H4 C420\n").unwrap();
    assert!(Plan::new(&h, Transform::default(), 47).is_err());
    for c in [
        Crop {
            x: usize::MAX,
            y: 0,
            width: 2,
            height: 2,
        },
        Crop {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        },
        Crop {
            x: 0,
            y: 0,
            width: 0,
            height: 2,
        },
    ] {
        assert!(
            Plan::new(
                &h,
                Transform {
                    crop: Some(c),
                    ..Default::default()
                },
                1024
            )
            .is_err()
        );
    }
    assert!(
        Plan::new(&h, Transform::default(), 48)
            .unwrap()
            .apply(&[0; 23], &mut [0; 24])
            .is_err()
    );
}
#[test]
fn all_formats_reflections_are_involutions() {
    for format in ["420", "422", "444"] {
        let h = Header::parse(format!("YUV4MPEG2 W8 H6 C{format}\n").as_bytes()).unwrap();
        let a: Vec<u8> = (0..h.frame_len().unwrap()).map(|i| i as u8).collect();
        for (horizontal, vertical) in [(true, false), (false, true), (true, true)] {
            let p = Plan::new(
                &h,
                Transform {
                    horizontal,
                    vertical,
                    crop: None,
                },
                1024,
            )
            .unwrap();
            let mut b = vec![0; a.len()];
            let mut c = b.clone();
            p.apply(&a, &mut b).unwrap();
            p.apply(&b, &mut c).unwrap();
            assert_eq!(a, c);
        }
    }
}
#[test]
fn deterministic_malformed_corpus_never_panics() {
    let mut state = 17u64;
    for n in 0..2000 {
        let mut data = b"YUV4MPEG2 ".to_vec();
        for _ in 0..(n % 100) {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            data.push((state >> 32) as u8);
        }
        data.push(b'\n');
        let _ = process(Cursor::new(data), Vec::new(), Transform::default(), 1024);
    }
}

#[test]
fn odd_y4m_identity_and_edge_crop_preserve_rounded_chroma() {
    let src = include_bytes!("fixtures/playback-errors/rotate-odd420.y4m");
    let mut out = Vec::new();
    let stats = process(Cursor::new(src), &mut out, Transform::default(), 4096).unwrap();
    assert_eq!(stats.frames, 3);
    assert_eq!(out, src);
    let header = Header::parse(b"YUV4MPEG2 W11 H11 C420jpeg\n").unwrap();
    let payload: Vec<u8> = (0..193).map(|n| n as u8).collect();
    let plan = Plan::new(
        &header,
        Transform {
            crop: Some(Crop {
                x: 8,
                y: 8,
                width: 3,
                height: 3,
            }),
            horizontal: true,
            vertical: true,
        },
        4096,
    )
    .unwrap();
    let mut cropped = vec![0; 17];
    plan.apply(&payload, &mut cropped).unwrap();
    assert_eq!(
        cropped,
        [
            120, 119, 118, 109, 108, 107, 98, 97, 96, 156, 155, 150, 149, 192, 191, 186, 185
        ]
    );
    assert!(
        Plan::new(
            &header,
            Transform {
                crop: Some(Crop {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 3
                }),
                ..Default::default()
            },
            4096
        )
        .is_err()
    );
}
