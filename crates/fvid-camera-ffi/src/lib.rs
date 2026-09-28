//! C boundary for the macOS host. Unsafe code is isolated from the FVid library.
use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraTick, LatestFrame, NativeCameraSource},
};
use std::{
    fs::File,
    io::BufReader,
    panic::{AssertUnwindSafe, catch_unwind},
};

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

/// Opens a UTF-8 path and reads the first frame to determine the output size.
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
    catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() || length == 0 || length > 32768 {
            return None;
        }
        let path = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, length) }).ok()?;
        let mut reader =
            NativeReader::software(BufReader::new(File::open(path).ok()?), budget).ok()?;
        if !reader.read_frame().ok()? {
            return None;
        }
        let [width, height] = reader.dimensions();
        let latest = LatestFrame::new(width, height, 64 << 20).ok()?;
        let (num, den) = reader.pixel_aspect();
        let pixel_aspect = [num, den];
        let duration_ns = reader
            .duration()
            .and_then(|d| u64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        reader.rewind().ok()?;
        Some(Box::into_raw(Box::new(CameraSource {
            source: NativeCameraSource::new(reader),
            latest,
            failed: false,
            duration_ns,
            pixel_aspect,
        })))
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
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
    if handle.is_null() || output.is_null() {
        return -1;
    }
    let source = unsafe { &mut *handle };
    if source.failed {
        return -1;
    }
    source.failed = true;
    let result = catch_unwind(AssertUnwindSafe(|| -> fvid::Result<bool> {
        let [w, h] = source.latest.dimensions();
        if length != w * h * 4 {
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
        _ => -1,
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
    if input.is_null() || output.is_null() || input_len > 64 << 20 || output_len > 64 << 20 {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        fvid::virtual_camera::fit_bgra(
            unsafe { std::slice::from_raw_parts(input, input_len) },
            [source.width as usize, source.height as usize],
            unsafe { std::slice::from_raw_parts_mut(output, output_len) },
            [target.width as usize, target.height as usize],
        )
        .map(|_| 1)
        .unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

/// Opaque clock owned by the serial camera producer.
#[unsafe(no_mangle)]
pub extern "C" fn fvid_camera_clock_open(now: u64) -> *mut fvid::virtual_camera::CameraClock {
    fvid::virtual_camera::CameraClock::new(30, 1, now)
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
    if handle.is_null()
        || input.is_null()
        || output.is_null()
        || input_len > 64 << 20
        || output_len > 64 << 20
    {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let source = unsafe { &*handle };
        fvid::virtual_camera::fit_bgra_aspect(
            unsafe { std::slice::from_raw_parts(input, input_len) },
            source.latest.dimensions(),
            source.pixel_aspect,
            unsafe { std::slice::from_raw_parts_mut(output, output_len) },
            [target.width as usize, target.height as usize],
        )
        .map(|_| 1)
        .unwrap_or(-1)
    }))
    .unwrap_or(-1)
}
