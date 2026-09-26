use fvid::container::mp4::{Limits, Mp4Reader, SkippedTrack};
use std::io::Cursor;

fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}
fn joined(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
/// A whole `tkhd`: unlike the fields a test cares about, the transform matrix and
/// the size the track is drawn at are fixed at the end of the header, so a short
/// one is a truncated file rather than a terse writer.
fn tkhd(id: u32, drawn: (u32, u32)) -> Vec<u8> {
    let mut body = vec![0u8; 84];
    body[12..16].copy_from_slice(&id.to_be_bytes());
    // Upright and unscaled, which is what a reader that divides the drawn size
    // by the coded one has to be given before either number means anything.
    for at in [40, 56] {
        body[at..at + 4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    }
    for (at, whole) in [(76, drawn.0), (80, drawn.1)] {
        body[at..at + 4].copy_from_slice(&(whole << 16).to_be_bytes());
    }
    atom(b"tkhd", &body)
}
/// A `trak` in a format this reader has no decoder for: a handler, one sample
/// description entry, and no sample tables at all. Files of this shape are
/// ordinary - QuickTime text subtitles, timecode tracks, ALAC audio - and each
/// of them used to cost the picture, because any unreadable track was fatal.
fn foreign_trak(id: u32, handler: &[u8; 4], codec: &[u8; 4], tables: &[Vec<u8>]) -> Vec<u8> {
    let mut entry = vec![0u8; 8];
    entry[6..8].copy_from_slice(&1u16.to_be_bytes());
    let dinf = atom(
        b"dinf",
        &atom(
            b"dref",
            &joined(&[words(&[0, 1]), atom(b"url ", &words(&[1]))]),
        ),
    );
    let mut parts = vec![atom(
        b"stsd",
        &joined(&[words(&[0, 1]), atom(codec, &entry)]),
    )];
    parts.extend_from_slice(tables);
    let stbl = atom(b"stbl", &joined(&parts));
    atom(
        b"trak",
        &joined(&[
            tkhd(id, (0, 0)),
            atom(
                b"mdia",
                &joined(&[
                    atom(b"mdhd", &words(&[0, 0, 0, 1000, 40, 0])),
                    atom(
                        b"hdlr",
                        &joined(&[words(&[0, 0]), handler.to_vec(), vec![0; 12]]),
                    ),
                    atom(b"minf", &joined(&[dinf, stbl])),
                ]),
            ),
        ]),
    )
}

fn fixture(wide: bool) -> Vec<u8> {
    fixture_traks(wide, &[], &[])
}

/// `extra` traks ride along with the picture; `configs` are boxes inside the
/// video sample entry, which is where a writer states the shape of a pixel.
fn fixture_traks(wide: bool, extra: &[Vec<u8>], configs: &[Vec<u8>]) -> Vec<u8> {
    let mdat = atom(b"mdat", &[1, 2, 3, 4, 5, 6]);
    let mut entry = vec![0u8; 78];
    entry[6..8].copy_from_slice(&1u16.to_be_bytes());
    entry[24..26].copy_from_slice(&16u16.to_be_bytes());
    entry[26..28].copy_from_slice(&16u16.to_be_bytes());
    entry.extend(atom(
        b"avcC",
        &[1, 66, 0, 10, 255, 225, 0, 1, 103, 1, 0, 1, 104],
    ));
    entry.extend(configs.concat());
    let stsd = atom(b"stsd", &joined(&[words(&[0, 1]), atom(b"avc1", &entry)]));
    let offsets = if wide {
        atom(
            b"co64",
            &joined(&[
                words(&[0, 2]),
                8u64.to_be_bytes().to_vec(),
                11u64.to_be_bytes().to_vec(),
            ]),
        )
    } else {
        atom(b"stco", &words(&[0, 2, 8, 11]))
    };
    let stbl = atom(
        b"stbl",
        &joined(&[
            stsd,
            atom(b"stsz", &words(&[0, 0, 3, 1, 2, 3])),
            atom(b"stts", &words(&[0, 2, 2, 10, 1, 20])),
            atom(b"ctts", &words(&[0x0100_0000, 2, 1, 10, 2, (-5i32) as u32])),
            atom(b"stsc", &words(&[0, 2, 1, 2, 1, 2, 1, 1])),
            offsets,
            atom(b"stss", &words(&[0, 2, 1, 3])),
        ]),
    );
    let dinf = atom(
        b"dinf",
        &atom(
            b"dref",
            &joined(&[words(&[0, 1]), atom(b"url ", &words(&[1]))]),
        ),
    );
    let mdia = atom(
        b"mdia",
        &joined(&[
            atom(b"mdhd", &words(&[0, 0, 0, 1000, 40, 0])),
            atom(
                b"hdlr",
                &joined(&[words(&[0, 0]), b"vide".to_vec(), vec![0; 12]]),
            ),
            atom(b"minf", &joined(&[dinf, stbl])),
        ]),
    );
    let trak = atom(
        b"trak",
        &joined(&[
            tkhd(7, (16, 16)),
            mdia,
            atom(b"edts", &atom(b"elst", &words(&[0, 1, 40, 5, 0x0001_0000]))),
        ]),
    );
    let mut traks = vec![trak];
    traks.extend_from_slice(extra);
    joined(&[
        mdat,
        atom(
            b"moov",
            &joined(
                &[atom(b"mvhd", &words(&[0, 0, 0, 1000, 40]))]
                    .into_iter()
                    .chain(traks)
                    .collect::<Vec<_>>(),
            ),
        ),
    ])
}

#[test]
fn indexed_packets_timestamps_sync_and_edits() {
    for wide in [false, true] {
        let mut mp4 = Mp4Reader::open(Cursor::new(fixture(wide)), Limits::default()).unwrap();
        assert_eq!(mp4.movie_timescale(), 1000);
        let track = &mp4.tracks()[0];
        assert_eq!(track.id, 7);
        assert_eq!(track.codec, *b"avc1");
        assert_eq!(
            track
                .samples
                .expanded()
                .expect("a video track indexes one record per sample")
                .iter()
                .map(|s| (s.offset, s.size, s.dts, s.pts, s.duration, s.sync))
                .collect::<Vec<_>>(),
            vec![
                (8, 1, 0, 10, 10, true),
                (9, 2, 10, 5, 10, false),
                (11, 3, 20, 15, 20, true)
            ]
        );
        assert_eq!(track.edits[0].duration, 40);
        assert_eq!(track.edits[0].media_time, 5);
        let mut packet = Vec::new();
        for (i, expected) in [vec![1], vec![2, 3], vec![4, 5, 6]].iter().enumerate() {
            mp4.read_packet(0, i, &mut packet).unwrap();
            assert_eq!(&packet, expected);
        }
        assert!(mp4.read_packet(0, 3, &mut packet).is_err());
        assert!(mp4.read_packet(1, 0, &mut packet).is_err());
    }
}
fn mutate_word(mut data: Vec<u8>, kind: &[u8; 4], offset: usize, value: u32) -> Vec<u8> {
    let p = data.windows(4).position(|w| w == kind).unwrap() + 4 + offset;
    data[p..p + 4].copy_from_slice(&value.to_be_bytes());
    data
}
#[test]
fn rejects_invalid_tables_offsets_and_external_references() {
    for data in [
        mutate_word(fixture(false), b"stco", 8, 0),
        mutate_word(fixture(false), b"stsz", 8, u32::MAX),
        mutate_word(fixture(false), b"stts", 8, 4),
        mutate_word(fixture(false), b"ctts", 8, 4),
        mutate_word(fixture(false), b"stsc", 8, 2),
        mutate_word(fixture(false), b"stsc", 12, 4),
        mutate_word(fixture(false), b"stss", 12, 1),
        mutate_word(fixture(false), b"mdhd", 12, 0),
        mutate_word(fixture(false), b"url ", 0, 0),
    ] {
        assert!(Mp4Reader::open(Cursor::new(data), Limits::default()).is_err());
    }
}
#[test]
fn every_truncation_is_rejected_and_limits_are_enforced() {
    let data = fixture(false);
    for end in 0..data.len() {
        assert!(
            Mp4Reader::open(Cursor::new(&data[..end]), Limits::default()).is_err(),
            "length {end}"
        );
    }
    for limits in [
        Limits {
            metadata_bytes: 1,
            ..Limits::default()
        },
        Limits {
            samples: 2,
            ..Limits::default()
        },
        Limits {
            tracks: 0,
            ..Limits::default()
        },
    ] {
        assert!(Mp4Reader::open(Cursor::new(&data), limits).is_err());
    }
    let mut mp4 = Mp4Reader::open(
        Cursor::new(data),
        Limits {
            packet_bytes: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(mp4.read_packet(0, 2, &mut Vec::new()).is_err());
}
#[test]
fn extended_box_size_and_fragment_rejection() {
    let mut data = fixture(false);
    let p = data.windows(4).position(|w| w == b"moov").unwrap() - 4;
    let old_size = data.len() - p;
    data.splice(
        p..p + 8,
        joined(&[
            words(&[1]),
            b"moov".to_vec(),
            ((old_size + 8) as u64).to_be_bytes().to_vec(),
        ]),
    );
    Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    let data = joined(&[fixture(false), atom(b"moof", &[])]);
    assert!(Mp4Reader::open(Cursor::new(data), Limits::default()).is_err());
}

#[test]
fn mutated_metadata_never_panics() {
    let original = fixture(false);
    for at in 14..original.len() {
        for replacement in [0, 1, 127, 255] {
            let mut mutated = original.clone();
            mutated[at] = replacement;
            let _ = Mp4Reader::open(
                Cursor::new(mutated),
                Limits {
                    samples: 32,
                    tracks: 4,
                    metadata_bytes: 4096,
                    packet_bytes: 64,
                },
            );
        }
    }
}

#[test]
fn invalid_box_reports_offset_size_and_remaining_bytes() {
    let bytes = [0, 0, 0, 100, b'm', b'd', b'a', b't'];
    let error = match Mp4Reader::open(Cursor::new(bytes), Limits::default()) {
        Ok(_) => panic!("oversized box accepted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("mdat"));
    assert!(error.contains("byte 0"));
    assert!(error.contains("declared size 100"));
    assert!(error.contains("remaining file bytes 8"));
}

#[test]
fn hevc_track_validates_hevc_configuration() {
    for codec in [b"hvc1", b"hev1"] {
        let mut data = fixture(false);
        for (old, new) in [(b"avc1", codec), (b"avcC", b"hvcC")] {
            let at = data.windows(4).position(|v| v == old).unwrap();
            data[at..at + 4].copy_from_slice(new);
        }
        let error =
            match fvid::playback_native::NativeReader::without_memory_limit(Cursor::new(data)) {
                Ok(_) => panic!("HEVC was incorrectly accepted by the AVC playback path"),
                Err(error) => error.to_string(),
            };
        assert!(error.contains("codec configuration"), "{error}");
        assert!(!error.contains("no supported"), "{error}");
    }
}

#[test]
fn an_unplayable_track_is_left_out_without_costing_the_picture() {
    for (handler, codec) in [
        (*b"sbtl", *b"dtxs"),
        (*b"time", *b"tmcd"),
        (*b"soun", *b"alac"),
    ] {
        let mut mp4 = Mp4Reader::open(
            Cursor::new(fixture_traks(
                false,
                &[foreign_trak(9, &handler, &codec, &[])],
                &[],
            )),
            Limits::default(),
        )
        .unwrap();
        let kept: Vec<String> = mp4
            .tracks()
            .iter()
            .map(|t| String::from_utf8_lossy(&t.codec).into_owned())
            .collect();
        assert_eq!(kept, ["avc1"], "{handler:?}/{codec:?} leaked in");
        // The remaining track keeps its index and its sample table.
        let mut packet = Vec::new();
        mp4.read_packet(0, 0, &mut packet).unwrap();
        assert_eq!(packet, vec![1]);
    }
}

/// The skip happens before the sample tables are built, so a foreign track
/// contributes nothing beyond its fourcc: a sample table the reader would refuse
/// outright sits in the file unnoticed.
#[test]
fn a_foreign_track_never_has_its_sample_table_read() {
    // 100 samples declared against a budget of 3 - fatal if it were indexed.
    let oversized = foreign_trak(9, b"soun", b"alac", &[atom(b"stsz", &words(&[0, 0, 100]))]);
    let mp4 = Mp4Reader::open(
        Cursor::new(fixture_traks(false, &[oversized], &[])),
        Limits {
            samples: 3,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(mp4.tracks().len(), 1);
    assert_eq!(mp4.tracks()[0].samples.len(), 3);
    // The track it set aside keeps its fourcc, so a caller can tell the coding it
    // has to implement apart from a file it failed to read.
    assert_eq!(
        mp4.refused(),
        [SkippedTrack {
            handler: *b"soun",
            codec: *b"alac",
        }]
    );
}

#[test]
fn a_file_of_only_unplayable_tracks_names_the_gap() {
    let mut data = fixture(false);
    for (old, new) in [(b"vide", b"time"), (b"avc1", b"tmcd")] {
        let at = data.windows(4).position(|v| v == old).unwrap();
        data[at..at + 4].copy_from_slice(new);
    }
    let error = Mp4Reader::open(Cursor::new(data), Limits::default())
        .err()
        .expect("a track with no decoder should not open");
    // The reader read every box of that track and then named the coding it has no
    // arm for, which is a gap of the decode layer: `Unsupported`, not `Invalid`.
    assert!(matches!(error, fvid::Error::Unsupported(_)), "{error}");
    let error = error.to_string();
    assert!(error.contains("no track in a codec"), "{error}");
    assert!(error.contains("tmcd"), "{error}");
    // An empty moov still reports what it holds rather than a codec gap.
    let empty = joined(&[
        atom(b"mdat", &[]),
        atom(b"moov", &atom(b"mvhd", &words(&[0, 0, 0, 1000, 40]))),
    ]);
    let error = Mp4Reader::open(Cursor::new(empty), Limits::default())
        .err()
        .expect("no tracks should not open")
        .to_string();
    assert!(error.contains("contains no tracks"), "{error}");
}

/// A file as `ffmpeg -c:s mov_text` writes it: an AVC picture, an AAC track and
/// a QuickTime text subtitle. Every reader in this repo refused the whole file
/// for the subtitle's sake; the picture now decodes end to end and the track is
/// indexed with it.
///
/// ```text
/// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=0.5 \
///        -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.5 -ac 2 \
///        -c:v libx264 -pix_fmt yuv420p -preset ultrafast -c:a aac -shortest base.mp4
/// ffmpeg -i base.mp4 -i two-lines.srt -c copy -c:s mov_text tests/fixtures/subtitles/mov-text.mp4
/// ```
#[test]
fn a_quicktime_text_track_does_not_spoil_the_picture() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/subtitles/mov-text.mp4");
    let mut mp4 = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
    let seen: Vec<String> = mp4
        .tracks()
        .iter()
        .map(|t| {
            format!(
                "{}/{}",
                String::from_utf8_lossy(&t.handler),
                String::from_utf8_lossy(&t.codec)
            )
        })
        .collect();
    assert_eq!(seen, ["vide/avc1", "soun/mp4a", "sbtl/tx3g"]);
    // The text track keeps its own clock and its own sample table: five
    // samples, of which the three empty ones are the cuts ffmpeg writes between
    // the two lines.
    let text = &mp4.tracks()[2];
    assert_eq!((text.timescale, text.samples.len()), (1_000_000, 5));
    assert_eq!(text.samples.get(1).unwrap().pts, 100_000);
    let mut packet = Vec::new();
    mp4.read_packet(2, 1, &mut packet).unwrap();
    assert_eq!(packet, b"\x00\nfirst line");
    let mut video =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(FIXTURE), Limits::default(), 1 << 20)
            .unwrap();
    let mut frames = 0;
    while video.read_frame().unwrap().is_some() {
        frames += 1;
    }
    // ffprobe's own count for the fixture.
    assert_eq!(frames, 12);
}

/// Anamorphic video stores fewer columns than it shows, and says so twice: `pasp`
/// gives the shape of one stored pixel, and the track header states the size the
/// picture is drawn at. Which of the two answers, and when neither does.
#[test]
fn a_track_reports_the_shape_of_its_pixels() {
    let shape = |data: Vec<u8>| {
        let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        mp4.tracks()[0].pixel_aspect
    };
    // The fixture is stored 16x16 and drawn 16x16.
    assert_eq!(shape(fixture(false)), (1, 1));
    // Drawn twice as wide as stored, the pixels are twice as wide as they are
    // tall; drawn twice as tall, they are half as wide.
    assert_eq!(
        shape(mutate_word(fixture(false), b"tkhd", 76, 32 << 16)),
        (2, 1)
    );
    assert_eq!(
        shape(mutate_word(fixture(false), b"tkhd", 80, 32 << 16)),
        (1, 2)
    );
    // A matrix that mirrors rather than turns is not one of the four quarter
    // turns, so it asks for nothing of the reader: the drawn size keeps its
    // whole answer, with no edge swapped.
    let mirrored = turned(fixture(false), [0xFFFF_0000, 0, 0, 0x0001_0000]);
    let mirrored = mutate_word(mirrored, b"tkhd", 76, 32 << 16);
    assert_eq!(shape(mirrored), (2, 1));
    // `pasp` travels with the stream rather than with the edit list, so it is
    // asked first and answers over a header that draws the same picture square.
    assert_eq!(
        shape(fixture_traks(false, &[], &[atom(b"pasp", &words(&[2, 1]))])),
        (2, 1)
    );
    // A box of ones, and a drawn size of nothing, both state nothing.
    assert_eq!(
        shape(fixture_traks(false, &[], &[atom(b"pasp", &words(&[0, 1]))])),
        (1, 1)
    );
    assert_eq!(shape(mutate_word(fixture(false), b"tkhd", 76, 0)), (1, 1));
}

/// Set the linear part of the `tkhd` transform: `m11`, `m12`, `m21`, `m22`.
fn turned(data: Vec<u8>, linear: [u32; 4]) -> Vec<u8> {
    let mut data = data;
    for (at, value) in [
        (40, linear[0]),
        (44, linear[1]),
        (52, linear[2]),
        (56, linear[3]),
    ] {
        data = mutate_word(data, b"tkhd", at, value);
    }
    data
}
/// The four turns a header can ask for, in 16.16 fixed point.
const QUARTER: [[u32; 4]; 4] = [
    [0x0001_0000, 0, 0, 0x0001_0000],
    [0, 0x0001_0000, 0xFFFF_0000, 0],
    [0xFFFF_0000, 0, 0, 0xFFFF_0000],
    [0, 0xFFFF_0000, 0x0001_0000, 0],
];

/// A header can turn the picture as well as stretch it, and the two statements
/// are about the same rectangle: the drawn size and the pixel shape both belong
/// to the picture once it has been turned.
#[test]
fn a_header_turns_the_picture_upright() {
    let read = |data: Vec<u8>| {
        let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        (mp4.tracks()[0].rotation, mp4.tracks()[0].pixel_aspect)
    };
    // Only a matrix that is exactly one of the four quarter turns is read as
    // one; the fixture's own header is the first of them.
    for (degrees, linear) in QUARTER.iter().enumerate() {
        assert_eq!(
            read(turned(fixture(false), *linear)),
            ((degrees * 90) as u16, (1, 1)),
            "{degrees}"
        );
    }
    // A turn with the axes scaled is not a turn either, and costs nothing.
    assert_eq!(
        read(turned(fixture(false), [0x0002_0000, 0, 0, 0x0002_0000])),
        (0, (1, 1))
    );
    // The shape a stored pixel has is carried through the turn, so a wide
    // pixel becomes a tall one in the picture the player stretches.
    assert_eq!(
        read(turned(
            fixture_traks(false, &[], &[atom(b"pasp", &words(&[2, 1]))]),
            QUARTER[1]
        )),
        (90, (1, 2))
    );
    assert_eq!(
        read(turned(
            fixture_traks(false, &[], &[atom(b"pasp", &words(&[2, 1]))]),
            QUARTER[2]
        )),
        (180, (2, 1))
    );
    // A picture stored 32x16 that a header turns sideways has its coded edges
    // swapped, so the drawn size has to be read against the turned pair: drawn
    // 16x32 is square pixels, drawn 32x32 is pixels twice as wide as tall.
    let wide = mutate_word(fixture(false), b"avc1", 24, (32 << 16) | 16);
    let drawn = |data: Vec<u8>, size: (u32, u32)| {
        let data = mutate_word(data, b"tkhd", 76, size.0 << 16);
        mutate_word(data, b"tkhd", 80, size.1 << 16)
    };
    assert_eq!(
        read(turned(drawn(wide.clone(), (16, 32)), QUARTER[1])),
        (90, (1, 1))
    );
    assert_eq!(
        read(turned(drawn(wide.clone(), (32, 32)), QUARTER[1])),
        (90, (2, 1))
    );
    // The same drawn size the picture is coded at says nothing by itself, and
    // a quarter turn must not turn that silence into a squared-off ratio: the
    // ratio of 32x16 drawn at 32x16 through a turn would otherwise be four.
    assert_eq!(
        read(turned(drawn(wide.clone(), (32, 16)), QUARTER[3])),
        (270, (1, 1))
    );
    // Without a turn the same pair does say nothing, for the ordinary reason.
    assert_eq!(read(drawn(wide, (32, 16))), (0, (1, 1)));
}

/// The same answer out of a file a real muxer wrote:
///
/// ```text
/// ffmpeg -f lavfi -i testsrc2=size=64x64:rate=25:duration=0.4 -vf setsar=2 \
///   -c:v libx264 -pix_fmt yuv420p tests/fixtures/display/par-2x1.mp4
/// ```
#[test]
fn an_anamorphic_file_states_the_shape_of_its_pixels() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/display/par-2x1.mp4");
    let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
    assert_eq!(mp4.tracks().len(), 1);
    let video = &mp4.tracks()[0];
    assert_eq!((video.width, video.height), (64, 64));
    assert_eq!(video.pixel_aspect, (2, 1));
}

/// `tests/fixtures/chapters/chapters.mp4`, made by muxing the Matroska file of
/// the same chapters across:
///
/// ```text
/// ffmpeg -i tests/fixtures/chapters/chapters.mkv -map 0 -c:v copy \
///   tests/fixtures/chapters/chapters.mp4
/// ```
/// The chapter list rides in `moov/udta/chpl`.
#[test]
fn a_movies_own_chapter_list_names_its_parts() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/chapters/chapters.mp4");
    let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
    let shown: Vec<(u64, &str)> = mp4
        .chapters()
        .iter()
        .map(|chapter| (chapter.start_ns, chapter.title.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (0, "Opening"),
            (1_000_000_000, "Глава 2"),
            (3_000_000_000, "End"),
        ]
    );
    // The chapter track the same muxer writes is not a track there is a decoder
    // for, so it is left out of the list the player walks.
    assert_eq!(
        mp4.tracks()
            .iter()
            .map(|track| track.handler)
            .collect::<Vec<_>>(),
        [*b"vide"]
    );
}

/// The very same list, written in the movie timescale the file itself states
/// (1 000 ticks to a second, four seconds of film) rather than the ten megahertz
/// `ffmpeg` counts in. Those times fit the film, so nothing has to be re-read.
#[test]
fn a_chapter_list_in_the_movies_own_clock_needs_no_second_reading() {
    let mut data = (*include_bytes!("fixtures/chapters/chapters.mp4")).to_vec();
    let at = data
        .windows(4)
        .position(|window| window == b"chpl")
        .expect("fixture states chapters");
    // Past the atom header, the version and flags, the four silent bytes and
    // the count, an entry is eight bytes of time, a length and the title.
    let mut at = at + 4 + 8 + 1;
    for seconds in [0u64, 1, 3] {
        for (slot, byte) in data[at..at + 8].iter_mut().enumerate() {
            *byte = (seconds * 1_000).to_be_bytes()[slot];
        }
        at += 8 + 1 + usize::from(data[at + 8]);
    }
    let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    let shown: Vec<u64> = mp4
        .chapters()
        .iter()
        .map(|chapter| chapter.start_ns / 1_000_000_000)
        .collect();
    assert_eq!(shown, [0, 1, 3]);
}

/// A chapter list this reader cannot walk costs the file nothing but its
/// chapters: the count claims more entries than the atom holds.
#[test]
fn a_truncated_chapter_list_leaves_the_movie_playable() {
    let mut data = (*include_bytes!("fixtures/chapters/chapters.mp4")).to_vec();
    let at = data
        .windows(4)
        .position(|window| window == b"chpl")
        .expect("fixture states chapters");
    data[at + 4 + 8] = 40;
    let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    assert_eq!(mp4.chapters().len(), 3, "what fits is still read");
    assert_eq!(mp4.tracks().len(), 1);
}

/// `tests/fixtures/tracks/named.mp4`, written by ffmpeg with a title on the
/// first audio and the first subtitle track and a language on two of the
/// audio ones:
///
/// ```text
/// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=0.4 \
///   -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.4 \
///   -f lavfi -i sine=frequency=880:sample_rate=32000:duration=0.4 \
///   -f lavfi -i sine=frequency=660:sample_rate=44100:duration=0.4 \
///   -i one.srt -i two.srt \
///   -map 0:v -map 1:a -map 2:a -map 3:a -map 4:0 -map 5:0 \
///   -c:v libx264 -profile:v baseline -pix_fmt yuv420p -preset ultrafast \
///   -c:a aac -ac 1 -c:s mov_text \
///   -metadata:s:a:0 title=Первая -metadata:s:a:0 language=rus \
///   -metadata:s:a:1 language=fre \
///   -metadata:s:s:0 title=Титры -metadata:s:s:0 language=rus \
///   tests/fixtures/tracks/named.mp4
/// ```
///
/// The picture, the last audio track and the last subtitle state neither: the
/// muxer marks them with the `und` that means nothing was said, and the reader
/// leaves both fields empty rather than showing those letters to the player.
#[test]
fn a_track_keeps_the_title_and_language_the_file_gives_it() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/tracks/named.mp4");
    let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
    let shown: Vec<(&[u8; 4], &str, &str)> = mp4
        .tracks()
        .iter()
        .map(|track| (&track.handler, track.name.as_str(), track.language.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (b"vide", "", ""),
            (b"soun", "Первая", "rus"),
            (b"soun", "", "fre"),
            (b"soun", "", ""),
            (b"sbtl", "Титры", "rus"),
            (b"sbtl", "", ""),
        ]
    );
}

/// A title that is not text costs the file nothing but its title: the language
/// the same track states is still read, and the other tracks keep theirs.
#[test]
fn a_title_that_is_not_text_costs_only_the_title() {
    let mut data = (*include_bytes!("fixtures/tracks/named.mp4")).to_vec();
    let at = data
        .windows(4)
        .position(|window| window == b"name")
        .expect("fixture titles a track");
    // Past the atom header lies the text, whose first byte here is the lead of
    // a two-byte sequence that cannot start a UTF-8 string.
    data[at + 8] = 0xff;
    let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    let tracks = mp4.tracks();
    assert_eq!(
        (tracks[1].name.as_str(), tracks[1].language.as_str()),
        ("", "rus")
    );
    assert_eq!(tracks[4].name, "Титры");
}

/// `mdhd` packs its three letters into sixteen bits and leaves the rest of a
/// shorter code at zero, which is padding rather than a letter: such a field
/// reads as nothing said, the way an absent one does.
#[test]
fn a_language_that_is_not_three_letters_reads_as_nothing_said() {
    let mut data = (*include_bytes!("fixtures/tracks/named.mp4")).to_vec();
    // The media header of the first audio track, found as the `mdhd` before the
    // title it carries: the last of its three five-bit groups goes to zero, so
    // what remains spells two letters and padding.
    let title = "Первая";
    let titled = data
        .windows(title.len())
        .position(|window| window == title.as_bytes())
        .expect("fixture titles a track");
    let at = data[..titled]
        .windows(4)
        .rposition(|window| window == b"mdhd")
        .expect("the titled track has a media header");
    data[at + 4 + 21] = 0;
    let mp4 = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    let tracks = mp4.tracks();
    assert_eq!(
        (tracks[1].name.as_str(), tracks[1].language.as_str()),
        ("Первая", "")
    );
    // The untouched tracks still state what they said.
    assert_eq!(tracks[2].language, "fre");
}
