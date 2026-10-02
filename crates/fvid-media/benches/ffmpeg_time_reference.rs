//! Explicit comparison benchmark against libavutil timestamp arithmetic.
use fvid_media::owned_time::{TimeBase, packet_nearest, rescale_nearest};
#[repr(C)]
#[derive(Clone, Copy)]
struct Rational {
    num: i32,
    den: i32,
}
#[link(name = "avutil")]
unsafe extern "C" {
    fn av_rescale_q(value: i64, source: Rational, target: Rational) -> i64;
    fn av_rescale_q_rnd(value: i64, source: Rational, target: Rational, rounding: i32) -> i64;
}
fn main() {
    let start = std::time::Instant::now();
    let mut count = 0;
    for (source, target) in [
        (1, 2),
        (2, 1),
        (48000, 1000),
        (44100, 90000),
        (90000, 44100),
        (i32::MAX, i32::MAX - 1),
    ] {
        for value in (-10000..=10000).chain([i64::MIN + 1, i64::MAX - 1, i64::MAX]) {
            let own = rescale_nearest(
                value,
                TimeBase {
                    numerator: 1,
                    denominator: source as u32,
                },
                TimeBase {
                    numerator: 1,
                    denominator: target as u32,
                },
            );
            // SAFETY: Plain scalar/rational ABI; libavutil owns no caller memory.
            let reference = unsafe {
                av_rescale_q(
                    value,
                    Rational {
                        num: 1,
                        den: source,
                    },
                    Rational {
                        num: 1,
                        den: target,
                    },
                )
            };
            match own {
                Ok(actual) => assert_eq!(actual, reference),
                Err(_) => assert_eq!(reference, i64::MIN),
            }
            let upper = fvid_media::owned_time::rescale_ceil(
                value,
                TimeBase {
                    numerator: 1,
                    denominator: source as u32,
                },
                TimeBase {
                    numerator: 1,
                    denominator: target as u32,
                },
            );
            // SAFETY: Scalar ABI, UP rounding mode (3).
            let reference_up = unsafe {
                av_rescale_q_rnd(
                    value,
                    Rational {
                        num: 1,
                        den: source,
                    },
                    Rational {
                        num: 1,
                        den: target,
                    },
                    3,
                )
            };
            match upper {
                Ok(actual) => assert_eq!(actual, reference_up),
                Err(_) => assert_eq!(reference_up, i64::MIN),
            }
            count += 2;
        }
    }
    // Packet timestamp sentinels use PASS_MINMAX rather than numeric rescaling.
    for value in [i64::MIN, i64::MAX] {
        let source = TimeBase {
            numerator: 1,
            denominator: 90000,
        };
        let target = TimeBase {
            numerator: 1,
            denominator: 44100,
        };
        let own = packet_nearest(value, value, 0, source, target).unwrap();
        // SAFETY: Same scalar ABI; NEAR_INF (5) | PASS_MINMAX (8192).
        let reference = unsafe {
            av_rescale_q_rnd(
                value,
                Rational { num: 1, den: 90000 },
                Rational { num: 1, den: 44100 },
                8197,
            )
        };
        assert_eq!(own, [reference, reference, 0]);
        count += 1;
    }
    println!(
        "{count} owned/libavutil conversions validated in {:?}",
        start.elapsed()
    );
}
