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
        let mut reader = NativeReader::new(BufReader::new(File::open(path).ok()?), budget).ok()?;
        if !reader.read_frame().ok()? {
            return None;
        }
        let [width, height] = reader.dimensions();
        let latest = LatestFrame::new(width, height, 64 << 20).ok()?;
        reader.rewind().ok()?;
        Some(Box::into_raw(Box::new(CameraSource {
            source: NativeCameraSource::new(reader),
            latest,
            failed: false,
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
