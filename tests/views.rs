use fvid::{Crop, Header, Plan, Transform};
use std::cell::Cell;
use std::io::{self, BufRead, Cursor, Read, Write};
use std::rc::Rc;

#[test]
fn transformed_views_borrow_source_and_match_materialized_frames() {
    for format in ["420", "422", "444"] {
        let header = Header::parse(format!("YUV4MPEG2 W8 H6 C{format}\n").as_bytes()).unwrap();
        let input: Vec<u8> = (0..header.frame_len().unwrap())
            .map(|i| (i * 17) as u8)
            .collect();
        for horizontal in [false, true] {
            for vertical in [false, true] {
                let plan = Plan::new(
                    &header,
                    Transform {
                        crop: Some(Crop {
                            x: 2,
                            y: 2,
                            width: 4,
                            height: 2,
                        }),
                        horizontal,
                        vertical,
                    },
                    4096,
                )
                .unwrap();
                let view = plan.view(&input).unwrap();
                assert_eq!((view.width(), view.height()), (4, 2));
                assert!(view.plane(3).is_none());
                let mut visible = Vec::new();
                for index in 0..3 {
                    let plane = view.plane(index).unwrap();
                    for row in 0..plane.height() {
                        let row = plane.row(row).unwrap();
                        // Prove each row aliases input bytes, rather than an owned copy.
                        let start = row.storage_bytes().as_ptr() as usize;
                        let base = input.as_ptr() as usize;
                        assert!(start >= base && start + plane.width() <= base + input.len());
                        assert_eq!(row.is_reversed(), horizontal);
                        visible.extend(row.pixels());
                    }
                    assert!(plane.row(plane.height()).is_none());
                    assert!(plane.row(usize::MAX).is_none());
                }
                let mut packed = vec![0; visible.len()];
                plan.apply(&input, &mut packed).unwrap();
                assert_eq!(visible, packed);
                assert!(plan.view(&input[..input.len() - 1]).is_err());
            }
        }
    }
}

#[test]
fn crop_and_flips_have_exact_original_sample_coordinates() {
    let header = Header::parse(b"YUV4MPEG2 W4 H4 C444\n").unwrap();
    let input: Vec<u8> = (0..48).collect();
    let plan = Plan::new(
        &header,
        Transform {
            crop: Some(Crop {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
            horizontal: true,
            vertical: true,
        },
        128,
    )
    .unwrap();
    let frame = plan.view(&input).unwrap();
    for (index, expected) in [[10, 9, 6, 5], [26, 25, 22, 21], [42, 41, 38, 37]]
        .iter()
        .enumerate()
    {
        let plane = frame.plane(index).unwrap();
        let pixels: Vec<_> = (0..2)
            .flat_map(|row| plane.row(row).unwrap().pixels())
            .collect();
        assert_eq!(&pixels, expected);
    }
}

struct TrackedInput {
    cursor: Cursor<Vec<u8>>,
    allocation: Rc<Cell<(usize, usize)>>,
}
impl Read for TrackedInput {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.allocation
            .set((buffer.as_ptr() as usize, buffer.len()));
        self.cursor.read(buffer)
    }
}
impl BufRead for TrackedInput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.cursor.fill_buf()
    }
    fn consume(&mut self, amount: usize) {
        self.cursor.consume(amount);
    }
}
struct TrackedOutput {
    allocation: Rc<Cell<(usize, usize)>>,
    borrowed_bytes: usize,
    bytes: Vec<u8>,
}
impl Write for TrackedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (base, length) = self.allocation.get();
        let address = bytes.as_ptr() as usize;
        if length != 0 && address >= base && address + bytes.len() <= base + length {
            self.borrowed_bytes += bytes.len();
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn cpu_pipeline_writes_original_rows_and_needs_only_one_frame_budget() {
    for format in ["420", "422", "444"] {
        for (vertical, horizontal) in [(false, false), (false, true), (true, false), (true, true)] {
            for crop in [
                None,
                Some(Crop {
                    x: 0,
                    y: 2,
                    width: 4,
                    height: 2,
                }),
                Some(Crop {
                    x: 2,
                    y: 0,
                    width: 2,
                    height: 4,
                }),
            ] {
                let header_line = format!("YUV4MPEG2 W4 H4 C{format}\n");
                let header = Header::parse(header_line.as_bytes()).unwrap();
                let length = header.frame_len().unwrap();
                let transform = Transform {
                    crop,
                    vertical,
                    horizontal,
                };
                let mut source = header_line.into_bytes();
                let mut expected = Vec::new();
                let plan = Plan::new(&header, transform, 1024).unwrap();
                for frame in 0..2 {
                    let pixels: Vec<_> = (0..length).map(|i| (i * 11 + frame * 37) as u8).collect();
                    source.extend_from_slice(b"FRAME\n");
                    source.extend_from_slice(&pixels);
                    let view = plan.view(&pixels).unwrap();
                    let output_length = (0..3)
                        .map(|i| {
                            let p = view.plane(i).unwrap();
                            p.width() * p.height()
                        })
                        .sum();
                    let mut packed = vec![0; output_length];
                    plan.apply(&pixels, &mut packed).unwrap();
                    expected.push(packed);
                }
                let allocation = Rc::new(Cell::new((0, 0)));
                let reader = TrackedInput {
                    cursor: Cursor::new(source.clone()),
                    allocation: allocation.clone(),
                };
                let mut writer = TrackedOutput {
                    allocation,
                    borrowed_bytes: 0,
                    bytes: Vec::new(),
                };
                let borrowed = !horizontal || crop.is_none_or(|c| c.width == header.width);
                let budget = length + if borrowed { 0 } else { expected[0].len() };
                let stats = fvid::process(reader, &mut writer, transform, budget).unwrap();
                assert_eq!(stats.controlled_memory_bytes, budget);
                assert_eq!(
                    writer.borrowed_bytes as u64,
                    if borrowed { stats.output_bytes } else { 0 }
                );
                let mut cursor = Cursor::new(&writer.bytes);
                let mut line = String::new();
                cursor.read_line(&mut line).unwrap();
                for packed in expected {
                    line.clear();
                    cursor.read_line(&mut line).unwrap();
                    assert_eq!(line, "FRAME\n");
                    let mut actual = vec![0; packed.len()];
                    cursor.read_exact(&mut actual).unwrap();
                    assert_eq!(actual, packed);
                }
                assert!(
                    fvid::process(Cursor::new(source), io::sink(), transform, budget - 1).is_err()
                );
            }
        }
    }
}
