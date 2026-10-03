//! Compatibility diagnostics for numeric backend errors, without libavutil.
//! Tagged codes are ABI values; OS errno messages come from the system library.
#[allow(dead_code)] // Also compiled by the explicit reference benchmark.
pub(crate) fn describe(code: i32) -> String {
    let tagged = match code {
        -1179861752 => "Bitstream filter not found",
        -558323010 => "Internal bug, should not have happened",
        -541545794 => "Internal bug, should not have happened",
        -1397118274 => "Buffer too small",
        -1128613112 => "Decoder not found",
        -1296385272 => "Demuxer not found",
        -1129203192 => "Encoder not found",
        -541478725 => "End of file",
        -1414092869 => "Immediate exit requested",
        -542398533 => "Generic error in an external library",
        -1279870712 => "Filter not found",
        -1094995529 => "Invalid data found when processing input",
        -1481985528 => "Muxer not found",
        -1414549496 => "Option not found",
        -1163346256 => "Not yet implemented in FFmpeg, patches welcome",
        -1330794744 => "Protocol not found",
        -1381258232 => "Stream not found",
        -1313558101 => "Unknown error occurred",
        -808465656 => "Server returned 400 Bad Request",
        -825242872 => "Server returned 401 Unauthorized (authorization failed)",
        -858797304 => "Server returned 403 Forbidden (access denied)",
        -875574520 => "Server returned 404 Not Found",
        -959591672 => "Server returned 429 Too Many Requests",
        -1482175736 => "Server returned 4XX Client Error, but not one of 40{0,1,3,4}",
        -1482175992 => "Server returned 5XX Server Error reply",
        -733130664 => "Experimental feature",
        -1668179713 => "Input changed",
        -1668179714 => "Output changed",
        _ => "",
    };
    if !tagged.is_empty() {
        return tagged.into();
    }
    system_message(code).unwrap_or_else(|| format!("Error number {code} occurred"))
}

#[cfg(unix)]
fn system_message(code: i32) -> Option<String> {
    let errno = code.checked_neg()?;
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY: Owned writable buffer; the POSIX interface respects its capacity.
    let result = unsafe { libc::strerror_r(errno, buffer.as_mut_ptr(), buffer.len()) };
    if result != 0 {
        return None;
    }
    // SAFETY: A successful strerror_r writes a terminated string into the buffer.
    Some(
        unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(not(unix))]
fn system_message(code: i32) -> Option<String> {
    let errno = code.checked_neg()?;
    // The backend's platforms without POSIX strerror_r use its portable errno
    // catalog instead of Win32 error messages (whose numbers differ from errno).
    Some(
        match errno {
            libc::E2BIG => "Argument list too long",
            libc::EACCES => "Permission denied",
            libc::EAGAIN => "Resource temporarily unavailable",
            libc::EBADF => "Bad file descriptor",
            libc::EBUSY => "Device or resource busy",
            libc::ECHILD => "No child processes",
            libc::EDEADLK => "Resource deadlock avoided",
            libc::EDOM => "Numerical argument out of domain",
            libc::EEXIST => "File exists",
            libc::EFAULT => "Bad address",
            libc::EFBIG => "File too large",
            libc::EILSEQ => "Illegal byte sequence",
            libc::EINTR => "Interrupted system call",
            libc::EINVAL => "Invalid argument",
            libc::EIO => "I/O error",
            libc::EISDIR => "Is a directory",
            libc::EMFILE => "Too many open files",
            libc::EMLINK => "Too many links",
            libc::ENAMETOOLONG => "File name too long",
            libc::ENFILE => "Too many open files in system",
            libc::ENODEV => "No such device",
            libc::ENOENT => "No such file or directory",
            libc::ENOEXEC => "Exec format error",
            libc::ENOLCK => "No locks available",
            libc::ENOMEM => "Cannot allocate memory",
            libc::ENOSPC => "No space left on device",
            libc::ENOSYS => "Function not implemented",
            libc::ENOTDIR => "Not a directory",
            libc::ENOTEMPTY => "Directory not empty",
            libc::ENOTTY => "Inappropriate I/O control operation",
            libc::ENXIO => "No such device or address",
            libc::EPERM => "Operation not permitted",
            libc::EPIPE => "Broken pipe",
            libc::ERANGE => "Result too large",
            libc::EROFS => "Read-only file system",
            libc::ESPIPE => "Illegal seek",
            libc::ESRCH => "No such process",
            libc::EXDEV => "Cross-device link",
            _ => return None,
        }
        .into(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn tagged_errno_and_unknown_codes_have_owned_diagnostics() {
        use super::describe;
        assert_eq!(describe(-541478725), "End of file");
        assert_eq!(
            describe(-1094995529),
            "Invalid data found when processing input"
        );
        assert_eq!(
            describe((-1668179713_i32) | (-1668179714_i32)),
            "Input changed"
        );
        assert_eq!(describe(-libc::EINVAL), "Invalid argument");
        assert_eq!(describe(i32::MIN), "Error number -2147483648 occurred");
        assert_eq!(describe(-123456789), "Error number -123456789 occurred");
    }
}
