//! Owned AV1 wedge mask construction from the normative master masks.
const ODD: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 6,
    18, 37, 53, 60, 63, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
    64, 64, 64, 64, 64, 64, 64, 64, 64,
];
const EVEN: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 4, 11,
    27, 46, 58, 62, 63, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
    64, 64, 64, 64, 64, 64, 64, 64, 64,
];
const VERTICAL: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 7,
    21, 43, 57, 62, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
    64, 64, 64, 64, 64, 64, 64, 64, 64,
];
const CODEBOOK: [[[usize; 3]; 16]; 3] = [
    [
        [2, 4, 4],
        [3, 4, 4],
        [4, 4, 4],
        [5, 4, 4],
        [0, 4, 2],
        [0, 4, 4],
        [0, 4, 6],
        [1, 4, 4],
        [2, 4, 2],
        [2, 4, 6],
        [5, 4, 2],
        [5, 4, 6],
        [3, 2, 4],
        [3, 6, 4],
        [4, 2, 4],
        [4, 6, 4],
    ],
    [
        [2, 4, 4],
        [3, 4, 4],
        [4, 4, 4],
        [5, 4, 4],
        [1, 2, 4],
        [1, 4, 4],
        [1, 6, 4],
        [0, 4, 4],
        [2, 4, 2],
        [2, 4, 6],
        [5, 4, 2],
        [5, 4, 6],
        [3, 2, 4],
        [3, 6, 4],
        [4, 2, 4],
        [4, 6, 4],
    ],
    [
        [2, 4, 4],
        [3, 4, 4],
        [4, 4, 4],
        [5, 4, 4],
        [0, 4, 2],
        [0, 4, 6],
        [1, 2, 4],
        [1, 6, 4],
        [2, 4, 2],
        [2, 4, 6],
        [5, 4, 2],
        [5, 4, 6],
        [3, 2, 4],
        [3, 6, 4],
        [4, 2, 4],
        [4, 6, 4],
    ],
];

fn master(direction: usize, y: usize, x: usize) -> i32 {
    fn oblique(y: usize, x: usize) -> i32 {
        let shift = 16 - (y as i32 + 1) / 2;
        let index = (x as i32 - shift).clamp(0, 63) as usize;
        if y & 1 == 0 { EVEN[index] } else { ODD[index] }
    }
    match direction {
        0 => VERTICAL[y],
        1 => VERTICAL[x],
        2 => oblique(x, y),
        3 => oblique(y, x),
        4 => 64 - oblique(y, 63 - x),
        5 => 64 - oblique(x, 63 - y),
        _ => unreachable!(),
    }
}
pub(super) fn wedge(size: [usize; 2], index: usize) -> [[i32; 32]; 32] {
    let [w, h] = size;
    let shape = if h > w {
        0
    } else if h < w {
        1
    } else {
        2
    };
    let [direction, dx, dy] = CODEBOOK[shape][index];
    let x = 32 - ((dx * w) >> 3);
    let y = 32 - ((dy * h) >> 3);
    let sum: i32 = (0..w).map(|i| master(direction, y, x + i)).sum::<i32>()
        + (1..h).map(|i| master(direction, y + i, x)).sum::<i32>();
    let length = (w + h - 1) as i32;
    let flip = (sum + length / 2) / length < 32;
    let mut out = [[0; 32]; 32];
    for row in 0..h {
        for col in 0..w {
            let v = master(direction, y + row, x + col);
            out[row][col] = if flip { 64 - v } else { v };
        }
    }
    out
}
