use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{Arc, atomic::Ordering},
};
#[path = "support/io_fault.rs"]
mod io_fault;
use io_fault::FaultReader;

const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/matroska-interrupted-copy.webm");

#[test]
fn interrupted_eof_check_preserves_successful_identity_copy() {
    let mut control = Vec::new();
    let expected = fvid_media::owned_matroska_copy::copy(
        &mut Cursor::new(VIDEO),
        &mut control,
        false,
        None,
        None,
    )
    .unwrap();
    assert_eq!(control, VIDEO);
    assert_eq!(expected.packets, 3);
    for chunk in [1, 7, usize::MAX] {
        let mut source =
            FaultReader::new(VIDEO, VIDEO.len() as u64, io::ErrorKind::Interrupted, chunk);
        let fired = Arc::clone(&source.fired);
        let mut copied = Vec::new();
        let event =
            fvid_media::owned_matroska_copy::copy(&mut source, &mut copied, false, None, None)
                .unwrap();
        assert!(fired.load(Ordering::Relaxed));
        assert_eq!(copied, VIDEO);
        assert_eq!(event, expected);
    }
}

#[test]
fn permanent_eof_check_error_is_preserved() {
    let mut source = FaultReader::new(
        VIDEO,
        VIDEO.len() as u64,
        io::ErrorKind::PermissionDenied,
        7,
    );
    let fired = Arc::clone(&source.fired);
    let error =
        fvid_media::owned_matroska_copy::copy(&mut source, &mut Vec::new(), false, None, None)
            .unwrap_err();
    assert!(fired.load(Ordering::Relaxed));
    assert_eq!(error.to_string(), "synthetic container I/O fault");
}

#[test]
fn cancellation_is_checked_after_interrupted_eof_verification() {
    struct CancelOnFault {
        source: FaultReader,
        cancel: fvid_control::CancelFlag,
    }
    impl Read for CancelOnFault {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let result = self.source.read(bytes);
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
            {
                self.cancel.cancel();
            }
            result
        }
    }
    impl Seek for CancelOnFault {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.source.seek(position)
        }
    }
    let cancel = fvid_control::CancelFlag::new();
    let mut source = CancelOnFault {
        source: FaultReader::new(VIDEO, VIDEO.len() as u64, io::ErrorKind::Interrupted, 7),
        cancel: cancel.clone(),
    };
    let error = fvid_media::owned_matroska_copy::copy(
        &mut source,
        &mut Vec::new(),
        false,
        Some(&cancel),
        None,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "media operation cancelled");
}
