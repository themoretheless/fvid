//! C boundary for the macOS host. Unsafe code is isolated from the FVid library.
use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraEndBehavior, CameraTick, LatestFrame, NativeCameraSource},
};
use std::{
    fs::File,
    io::BufReader,
    panic::{AssertUnwindSafe, catch_unwind},
};

thread_local! {
    static LAST_ERROR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}
fn set_error(message: impl AsRef<str>) {
    LAST_ERROR.with(|value| {
        let message = message.as_ref();
        let mut end = message.len().min(4096);
        while !message.is_char_boundary(end) { end -= 1; }
        let mut value = value.borrow_mut(); value.clear(); value.push_str(&message[..end]);
    });
}
/// UTF-8 diagnostic for the last source open/frame/fit operation on this thread.
/// Returns required bytes, without a NUL terminator. A null output queries size.
/// Read immediately after failure, on the same thread. Reading does not clear it.
/// # Safety
/// Non-null output must reference capacity writable bytes, with no live alias.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_error(output: *mut u8, capacity: usize) -> usize {
    LAST_ERROR.with(|value| {
        let value = value.borrow();
        if !output.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), output, capacity.min(value.len())); }
        }
        value.len()
    })
}
fn status(result: std::thread::Result<fvid::Result<()>>) -> i32 {
    match result {
        Ok(Ok(())) => 1,
        Ok(Err(error)) => { set_error(error.to_string()); -1 },
        Err(_) => { set_error("camera frame conversion panicked"); -1 },
    }
}

type Source = NativeCameraSource<BufReader<File>>;
pub struct CameraSource {
    source: Source,
    latest: LatestFrame,
    failed: bool,
    duration_ns: u64,
    pixel_aspect: [u32; 2],
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct CameraSize {
    pub width: u32,
    pub height: u32,
}

/// Opens a UTF-8 path and reads the first frame to determine the visible output size.
/// Container crop and rotation are reflected in size and subsequent BGRA frames.
/// Returns null on error; budget covers decoder/RGB storage. A separate BGRA
/// buffer is capped at 64 MiB. No OS camera is installed or opened here.
/// # Safety
/// `path` must reference `length` readable bytes for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_open(
    path: *const u8,
    length: usize,
    budget: usize,
) -> *mut CameraSource {
    set_error("");
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<*mut CameraSource, String> {
        if path.is_null() || length == 0 || length > 32768 {
            return Err("invalid camera source path".into());
        }
        let path = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, length) })
            .map_err(|error| error.to_string())?;
        let mut reader = NativeReader::software(
            BufReader::new(File::open(path).map_err(|error| error.to_string())?), budget)
            .map_err(|error| error.to_string())?;
        if !reader.read_frame().map_err(|error| error.to_string())? {
            return Err("camera source contains no video frames".into());
        }
        let [width, height] = fvid::virtual_camera::visible_dimensions(reader.dimensions(), reader.insets())
            .map_err(|error| error.to_string())?;
        let latest = LatestFrame::new(width, height, 64 << 20).map_err(|error| error.to_string())?;
        let (num, den) = reader.pixel_aspect();
        let duration_ns = reader.duration().and_then(|d| u64::try_from(d.as_nanos()).ok()).unwrap_or(0);
        reader.rewind().map_err(|error| error.to_string())?;
        Ok(Box::into_raw(Box::new(CameraSource {
            source: NativeCameraSource::new(reader), latest, failed: false, duration_ns,
            pixel_aspect: [num,den],
        })))
    }));
    match result {
        Ok(Ok(handle)) => handle,
        Ok(Err(error)) => { set_error(error); std::ptr::null_mut() },
        Err(_) => { set_error("camera source initialization panicked"); std::ptr::null_mut() },
    }
}
/// # Safety
/// `handle` must be null or a live handle returned by `fvid_camera_open`.
/// Calls for the same handle must be serialized, including close.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_size(handle: *const CameraSource) -> CameraSize {
    if handle.is_null() {
        return CameraSize::default();
    }
    let [width, height] = unsafe { &*handle }.latest.dimensions();
    CameraSize {
        width: width as u32,
        height: height as u32,
    }
}
/// Set EOF policy: 0 holds the last frame, 1 repeats the file.
/// Returns 1 on success, -1 on invalid input or a failed handle. Invalid policy
/// values leave the source unchanged and do not poison it.
/// # Safety
/// `handle` must be null or a live, exclusively borrowed source handle.
/// Calls for the same handle must be serialized, including close.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_set_loop(handle: *mut CameraSource, enabled: u32) -> i32 {
    set_error("");
    if handle.is_null() { set_error("null camera handle"); return -1; }
    let behavior = match enabled {
        0 => CameraEndBehavior::Hold,
        1 => CameraEndBehavior::Loop,
        _ => { set_error("camera loop policy must be 0 or 1"); return -1; }
    };
    let source = unsafe { &mut *handle };
    if source.failed { set_error("camera source failed; close and reopen it"); return -1; }
    source.source.set_end_behavior(behavior);
    1
}

/// Copies tightly packed BGRA for a media position. Returns 1 on success and -1 on error. An error poisons the handle: close and reopen it.
/// Host timestamp and sequence must increase even when media time seeks back.
/// # Safety
/// `handle` must be live and exclusively borrowed for this call. `output` must
/// reference `length` writable bytes, disjoint from the handle and its storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_frame(
    handle: *mut CameraSource,
    media_ns: u64,
    host_ns: u64,
    sequence: u64,
    output: *mut u8,
    length: usize,
) -> i32 {
    set_error("");
    if handle.is_null() || output.is_null() {
        set_error("null camera handle or frame buffer");
        return -1;
    }
    let source = unsafe { &mut *handle };
    if source.failed {
        set_error("camera source failed; close and reopen it");
        return -1;
    }
    source.failed = true;
    let result = catch_unwind(AssertUnwindSafe(|| -> fvid::Result<bool> {
        let [w, h] = source.latest.dimensions();
        if length != w * h * 4 {
            set_error("invalid camera frame buffer length");
            return Ok(false);
        }
        if !source.source.publish(
            CameraTick {
                sequence,
                host_time_ns: host_ns,
                media_time_ns: media_ns,
            },
            &source.latest,
        )? {
            return Ok(false);
        }
        let output = unsafe { std::slice::from_raw_parts_mut(output, length) };
        Ok(source.latest.copy_latest(None, output)?.is_some())
    }));
    match result {
        Ok(Ok(true)) => {
            source.failed = false;
            1
        }
        Ok(Err(error)) => { set_error(error.to_string()); -1 },
        Ok(Ok(false)) => {
            if LAST_ERROR.with(|error| error.borrow().is_empty()) { set_error("camera source produced no frame"); }
            -1
        },
        Err(_) => { set_error("camera source decoding panicked"); -1 },
    }
}
/// # Safety
/// `handle` must be null or a live, exclusively owned handle from open. A nonnull
/// handle is invalid after this call and must never be reused or closed twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_close(handle: *mut CameraSource) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// # Safety
/// Input and output must be disjoint readable/writable regions of the specified
/// lengths. Dimensions must describe packed BGRA. No pointers are retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_fit(
    input: *const u8,
    input_len: usize,
    source: CameraSize,
    output: *mut u8,
    output_len: usize,
    target: CameraSize,
) -> i32 {
    set_error("");
    if input.is_null() || output.is_null() || input_len > 64 << 20 || output_len > 64 << 20 {
        set_error("invalid camera conversion buffer or handle");
        return -1;
    }
    status(catch_unwind(AssertUnwindSafe(|| {
        fvid::virtual_camera::fit_bgra(
            unsafe { std::slice::from_raw_parts(input, input_len) },
            [source.width as usize, source.height as usize],
            unsafe { std::slice::from_raw_parts_mut(output, output_len) },
            [target.width as usize, target.height as usize],
        )
    })))
}

/// Opaque clock owned by the serial camera producer.
#[unsafe(no_mangle)]
pub extern "C" fn fvid_camera_clock_open(now: u64) -> *mut fvid::virtual_camera::CameraClock {
    fvid_camera_clock_open_rate(30, 1, now)
}

/// Create an exact rational-rate camera clock; null on invalid rate.
#[unsafe(no_mangle)]
pub extern "C" fn fvid_camera_clock_open_rate(numerator: u32, denominator: u32, now: u64) -> *mut fvid::virtual_camera::CameraClock {
    fvid::virtual_camera::CameraClock::new(numerator, denominator, now)
        .map(|clock| Box::into_raw(Box::new(clock)))
        .unwrap_or(std::ptr::null_mut())
}
#[repr(C)]
#[derive(Default)]
pub struct ClockTick {
    pub sequence: u64,
    pub host_ns: u64,
    pub media_ns: u64,
}
/// Returns 1 for a tick, 0 for a skipped slot, -1 for invalid time/pointers.
/// # Safety
/// Clock must be live and exclusively borrowed; output must be writable and disjoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_clock_poll(
    clock: *mut fvid::virtual_camera::CameraClock,
    now: u64,
    output: *mut ClockTick,
) -> i32 {
    if clock.is_null() || output.is_null() {
        return -1;
    }
    match unsafe { &mut *clock }.poll(now) {
        Ok(Some(tick)) => {
            unsafe {
                *output = ClockTick {
                    sequence: tick.sequence,
                    host_ns: tick.host_time_ns,
                    media_ns: tick.media_time_ns,
                };
            }
            1
        }
        Ok(None) => 0,
        Err(_) => -1,
    }
}
/// Commands: 0 seek to value ns, 1 pause (value != 0), 2 loop duration (0 disables).
/// # Safety
/// Clock must be null or live and exclusively borrowed; calls must be serialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_clock_control(
    clock: *mut fvid::virtual_camera::CameraClock,
    command: u32,
    value: u64,
    now: u64,
) -> i32 {
    if clock.is_null() {
        return -1;
    }
    let clock = unsafe { &mut *clock };
    let result = match command {
        0 => clock.seek(value, now),
        1 => clock.set_paused(value != 0, now),
        2 => clock.set_loop_duration((value != 0).then_some(value), now),
        _ => return -1,
    };
    result.map(|_| 1).unwrap_or(-1)
}
/// # Safety
/// Clock must be null or a uniquely owned live handle; it is invalid after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_clock_close(clock: *mut fvid::virtual_camera::CameraClock) {
    if !clock.is_null() {
        drop(unsafe { Box::from_raw(clock) });
    }
}

/// Duration in nanoseconds, or zero when unknown. Does not scan the entire file.
/// # Safety
/// Handle must be null or live; all operations on the handle must be serialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_duration(handle: *const CameraSource) -> u64 {
    if handle.is_null() {
        0
    } else {
        unsafe { &*handle }.duration_ns
    }
}

/// Fit this source's BGRA samples into square output pixels, preserving display aspect.
/// # Safety
/// Handle must be live and serialized; input/output must be disjoint readable/writable
/// regions of their stated lengths, also disjoint from the handle and its storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fvid_camera_fit_source(
    handle: *const CameraSource,
    input: *const u8,
    input_len: usize,
    output: *mut u8,
    output_len: usize,
    target: CameraSize,
) -> i32 {
    set_error("");
    if handle.is_null()
        || input.is_null()
        || output.is_null()
        || input_len > 64 << 20
        || output_len > 64 << 20
    {
        set_error("invalid camera conversion buffer or handle");
        return -1;
    }
    status(catch_unwind(AssertUnwindSafe(|| {
        let source = unsafe { &*handle };
        fvid::virtual_camera::fit_bgra_aspect(
            unsafe { std::slice::from_raw_parts(input, input_len) },
            source.latest.dimensions(),
            source.pixel_aspect,
            unsafe { std::slice::from_raw_parts_mut(output, output_len) },
            [target.width as usize, target.height as usize],
        )
    })))
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    fn message() -> String {
        let count = unsafe { fvid_camera_error(std::ptr::null_mut(), 0) };
        let mut bytes = vec![0; count];
        assert_eq!(unsafe { fvid_camera_error(bytes.as_mut_ptr(), bytes.len()) }, count);
        String::from_utf8(bytes).unwrap()
    }
    #[test]
    fn diagnostics_are_bounded_utf8_and_thread_local() {
        set_error("Ошибка камеры".repeat(1000));
        let original = message();
        assert!(original.len() <= 4096);
        let mut tiny = [0;3];
        assert_eq!(unsafe { fvid_camera_error(tiny.as_mut_ptr(),tiny.len()) }, original.len());
        assert_eq!(&tiny, &original.as_bytes()[..3]);
        std::thread::spawn(|| { assert!(message().is_empty()); set_error("other thread"); }).join().unwrap();
        assert_eq!(message(),original);
        set_error(""); assert!(message().is_empty());
    }
    #[test]
    fn invalid_source_and_buffers_report_errors_without_unwinding() {
        let opened = unsafe { fvid_camera_open(std::ptr::null(),0,1024) };
        assert!(opened.is_null()); assert!(message().contains("path"));
        let result = unsafe { fvid_camera_frame(std::ptr::null_mut(),0,0,0,std::ptr::null_mut(),0) };
        assert_eq!(result,-1); assert!(message().contains("handle"));
        let result = unsafe { fvid_camera_fit(std::ptr::null(),0,CameraSize::default(),std::ptr::null_mut(),0,CameraSize::default()) };
        assert_eq!(result,-1); assert!(message().contains("buffer"));
    }
}

#[cfg(test)]
mod crop_tests {
    use super::*;
    #[test]
    fn cropped_and_rotated_files_expose_visible_bgra_and_rewind_pixels() {
        for fixture in ["display/crops.mkv", "display/vp9-rot90.mkv"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(fixture);
            let name = path.to_str().unwrap();
            let mut reader = NativeReader::software(BufReader::new(File::open(&path).unwrap()), usize::MAX).unwrap();
            assert!(reader.read_frame().unwrap());
            let [w,h] = reader.dimensions();
            let [l,t,r,b] = reader.insets().map(|n| n as usize);
            let visible = [w-l-r,h-t-b];
            let mut expected = Vec::new();
            for y in t..h-b {
                for x in l..w-r {
                    let pixel = &reader.rgb()[(y*w+x)*3..(y*w+x)*3+3];
                    expected.extend_from_slice(&[pixel[2],pixel[1],pixel[0],255]);
                }
            }
            let handle = unsafe { fvid_camera_open(name.as_ptr(), name.len(), 256 << 20) };
            assert!(!handle.is_null());
            struct Close(*mut CameraSource);
            impl Drop for Close { fn drop(&mut self) { unsafe { fvid_camera_close(self.0) }; } }
            let _close = Close(handle);
            let size = unsafe { fvid_camera_size(handle) };
            assert_eq!([size.width as usize,size.height as usize],visible);
            let mut output = vec![0;expected.len()];
            for (sequence, media_ns) in [0,1_000_000_000_000,0].into_iter().enumerate() {
                assert_eq!(unsafe { fvid_camera_frame(handle,media_ns,sequence as u64+1,sequence as u64,output.as_mut_ptr(),output.len()) },1);
                if media_ns == 0 { assert_eq!(output, expected); }
            }
        }
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    #[test]
    fn c_loop_switch_is_reversible_and_invalid_values_preserve_source() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/avc-bypass-main10.mp4");
        let name = path.to_str().unwrap();
        let handle = unsafe { fvid_camera_open(name.as_ptr(),name.len(),16<<20) };
        assert!(!handle.is_null());
        struct Close(*mut CameraSource);
        impl Drop for Close { fn drop(&mut self) { unsafe { fvid_camera_close(self.0) }; } }
        let _close = Close(handle);
        let mut first = vec![0;64*64*4];
        let mut current = first.clone();
        assert_eq!(unsafe { fvid_camera_frame(handle,0,1,0,first.as_mut_ptr(),first.len()) },1);
        assert_eq!(unsafe { fvid_camera_set_loop(handle,1) },1);
        assert_eq!(unsafe { fvid_camera_set_loop(handle,2) },-1);
        // The fixture contains eight 30-fps frames, ending at 266666667 ns.
        let duration = 266_666_667;
        assert_eq!(unsafe { fvid_camera_frame(handle,duration,2,1,current.as_mut_ptr(),current.len()) },1);
        assert_eq!(current,first);
        assert_eq!(unsafe { fvid_camera_set_loop(handle,0) },1);
        assert_eq!(unsafe { fvid_camera_frame(handle,duration,3,2,current.as_mut_ptr(),current.len()) },1);
        assert_ne!(current,first);
        assert_eq!(unsafe { fvid_camera_set_loop(handle,1) },1);
        assert_eq!(unsafe { fvid_camera_frame(handle,2*duration,4,3,current.as_mut_ptr(),current.len()) },1);
        assert_eq!(current,first);
        assert_eq!(unsafe { fvid_camera_set_loop(std::ptr::null_mut(),1) },-1);
    }
}
