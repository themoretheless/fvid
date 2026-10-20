use fvid_control::error::Error;
use std::error::Error as _;

#[test]
fn io_error_keeps_its_typed_source() {
    let error = Error::from(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "synthetic permission failure",
    ));
    let source = error.source().expect("I/O cause must remain accessible");
    let io = source.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "synthetic permission failure");
}

#[test]
fn message_errors_have_no_invented_source() {
    for error in [
        Error::Invalid("bad input".into()),
        Error::Unsupported("unavailable".into()),
        Error::Gpu("device lost".into()),
    ] {
        assert!(error.source().is_none());
    }
}
