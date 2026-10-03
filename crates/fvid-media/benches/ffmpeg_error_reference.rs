//! Explicit compatibility benchmark; production diagnostics never call libav.
#[path = "../src/owned_backend_error.rs"]
mod owned_backend_error;
#[link(name = "avutil")]
unsafe extern "C" {
    fn av_strerror(code: i32, output: *mut libc::c_char, size: usize) -> i32;
}
fn main() {
    let mut codes = (-2048..=2048).collect::<Vec<i32>>();
    for tag in [
        [248, b'B', b'S', b'F'],
        *b"BUG!",
        *b"BUG ",
        *b"BUFS",
        [248, b'D', b'E', b'C'],
        [248, b'D', b'E', b'M'],
        [248, b'E', b'N', b'C'],
        *b"EOF ",
        *b"EXIT",
        *b"EXT ",
        [248, b'F', b'I', b'L'],
        *b"INDA",
        [248, b'M', b'U', b'X'],
        [248, b'O', b'P', b'T'],
        *b"PAWE",
        [248, b'P', b'R', b'O'],
        [248, b'S', b'T', b'R'],
        *b"UNKN",
        [248, b'4', b'0', b'0'],
        [248, b'4', b'0', b'1'],
        [248, b'4', b'0', b'3'],
        [248, b'4', b'0', b'4'],
        [248, b'4', b'2', b'9'],
        [248, b'4', b'X', b'X'],
        [248, b'5', b'X', b'X'],
    ] {
        codes.push(-i32::from_le_bytes(tag));
    }
    codes.extend([
        -0x2bb2afa8,
        -0x636e6701,
        -0x636e6702,
        (-0x636e6701_i32) | (-0x636e6702_i32),
        i32::MIN,
        i32::MAX,
        -123456789,
        -1668179715,
    ]);
    for &code in &codes {
        let mut buffer = [0 as libc::c_char; 256];
        // SAFETY: Writable owned buffer with its correct capacity.
        unsafe {
            av_strerror(code, buffer.as_mut_ptr(), buffer.len());
        }
        // SAFETY: The reference formatter terminates inside the supplied buffer.
        let reference = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
        assert_eq!(
            owned_backend_error::describe(code),
            reference,
            "error code {code}"
        );
    }
    println!("{} numeric error descriptions match libavutil", codes.len());
}
