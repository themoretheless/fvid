//! Finite HTTP media input backed by a private temporary file, independent of libav.
//! Download precedes decode. The owner keeps the source available for audio reopen
//! and arbitrary seeks, and removes it when the input is no longer needed.
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct DownloadOptions {
    pub timeout: Duration,
    pub max_bytes: Option<u64>,
    pub cancel: Option<CancelFlag>,
    pub progress: Option<ProgressHook>,
}
impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            max_bytes: None,
            cancel: None,
            progress: None,
        }
    }
}
#[derive(Debug)]
pub struct DownloadedInput {
    directory: PathBuf,
    path: PathBuf,
    bytes: u64,
}
impl Drop for DownloadedInput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
impl DownloadedInput {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn download(url: &str, options: &DownloadOptions) -> Result<Self> {
        if crate::input_policy::active() {
            return Err("HTTP input is disabled in standalone input scope".into());
        }
        cancelled(options)?;
        let url = reqwest::Url::parse(url).map_err(|_| "invalid HTTP media URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("owned HTTP input requires http or https".into());
        }
        if options.timeout.is_zero() {
            return Err("HTTP input timeout must be positive".into());
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(options.timeout)
            .connect_timeout(options.timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| error.without_url().to_string())?;
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .map_err(|error| error.without_url().to_string())?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "HTTP media input returned status {}",
                response.status().as_u16()
            ));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|encoding| !encoding.as_bytes().eq_ignore_ascii_case(b"identity"))
        {
            return Err("HTTP media input requires an unencoded response body".into());
        }
        let length = response.content_length();
        let extension = Path::new(url.path())
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("media");
        let extension =
            if extension.len() <= 16 && extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
                extension
            } else {
                "media"
            };
        Self::from_body(response, length, extension, options)
    }
    fn from_body(
        mut body: impl Read,
        length: Option<u64>,
        extension: &str,
        options: &DownloadOptions,
    ) -> Result<Self> {
        cancelled(options)?;
        if length
            .zip(options.max_bytes)
            .is_some_and(|(length, limit)| length > limit)
        {
            return Err("HTTP media input exceeds download byte limit".into());
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fvid-http-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).map_err(|e| e.to_string())?;
        let mut input = Self {
            path: directory.join(format!("source.{extension}")),
            directory,
            bytes: 0,
        };
        let mut file = File::create(&input.path).map_err(|e| e.to_string())?;
        let mut scratch = [0u8; 64 * 1024];
        let result: Result<()> = (|| {
            loop {
                cancelled(options)?;
                let count = body
                    .read(&mut scratch)
                    .map_err(|e| format!("HTTP media body read: {e}"))?;
                if count == 0 {
                    break;
                }
                input.bytes = input
                    .bytes
                    .checked_add(count as u64)
                    .ok_or("HTTP media byte count overflow")?;
                if options.max_bytes.is_some_and(|limit| input.bytes > limit) {
                    return Err("HTTP media input exceeds download byte limit".into());
                }
                file.write_all(&scratch[..count])
                    .map_err(|e| e.to_string())?;
                if let Some(progress) = &options.progress {
                    progress.emit(ProgressEvent {
                        packets: 0,
                        payload_bytes: input.bytes,
                        done: false,
                    });
                }
            }
            cancelled(options)?;
            if length.is_some_and(|length| input.bytes != length) {
                return Err("truncated HTTP media response body".into());
            }
            if input.bytes == 0 {
                return Err("HTTP media response body is empty".into());
            }
            file.flush().map_err(|e| e.to_string())?;
            Ok(())
        })();
        // Close before owner cleanup, including failure paths on Windows.
        drop(file);
        result?;
        if let Some(progress) = &options.progress {
            progress.emit(ProgressEvent {
                packets: 0,
                payload_bytes: input.bytes,
                done: true,
            });
        }
        Ok(input)
    }
}
pub fn recognizes(url: &str) -> bool {
    url.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}
fn cancelled(options: &DownloadOptions) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(CancelFlag::is_cancelled)
    {
        Err("HTTP media download cancelled".into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const VIDEO: &[u8] = include_bytes!("../../../tests/fixtures/playback-errors/rotate-grid.y4m");
    #[test]
    fn finite_body_is_seekable_and_retained_until_owner_drop() {
        let input =
            DownloadedInput::from_body(VIDEO, Some(VIDEO.len() as u64), "y4m", &Default::default())
                .unwrap();
        assert_eq!(input.bytes(), VIDEO.len() as u64);
        assert_eq!(std::fs::read(input.path()).unwrap(), VIDEO);
        assert_eq!(
            crate::owned_y4m_decode::decode_video(input.path())
                .unwrap()
                .video_frames,
            3
        );
        let path = input.path().to_owned();
        let directory = input.directory.clone();
        drop(input);
        assert!(!path.exists());
        assert!(!directory.exists());
    }
    #[test]
    fn unknown_length_progress_cancellation_and_limits_are_checked() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let report = events.clone();
        let options = DownloadOptions {
            progress: Some(ProgressHook::new(move |event| {
                report.lock().unwrap().push(event)
            })),
            ..Default::default()
        };
        let input = DownloadedInput::from_body(VIDEO, None, "y4m", &options).unwrap();
        let events = events.lock().unwrap();
        assert!(events.last().unwrap().done);
        assert_eq!(events.last().unwrap().payload_bytes, input.bytes());
        for length in [0, VIDEO.len() as u64 + 1] {
            assert!(
                DownloadedInput::from_body(VIDEO, Some(length), "y4m", &Default::default())
                    .unwrap_err()
                    .contains("truncated")
            );
        }
        let limited = DownloadOptions {
            max_bytes: Some(1),
            ..Default::default()
        };
        assert!(
            DownloadedInput::from_body(VIDEO, None, "y4m", &limited)
                .unwrap_err()
                .contains("byte limit")
        );
        let flag = CancelFlag::new();
        let trigger = flag.clone();
        let options = DownloadOptions {
            cancel: Some(flag),
            progress: Some(ProgressHook::new(move |_| trigger.cancel())),
            ..Default::default()
        };
        assert!(
            DownloadedInput::from_body(VIDEO, None, "y4m", &options)
                .unwrap_err()
                .contains("cancelled")
        );
    }
    #[test]
    fn standalone_scope_refuses_before_network_and_other_schemes_are_excluded() {
        crate::with_standalone_inputs(|| {
            assert!(
                DownloadedInput::download("https://invalid.example/video", &Default::default())
                    .unwrap_err()
                    .contains("standalone")
            )
        });
        assert!(!recognizes("rtsp://server/video"));
        assert!(recognizes("HTTPS://server/video"));
    }
    #[test]
    #[ignore = "explicit loopback HTTP integration; ordinary tests require no network"]
    fn live_loopback_download_redirect_chunking_and_truncation() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                if request.starts_with("GET /redirect") {
                    write!(socket,"HTTP/1.1 302 Found\r\nLocation: /video.y4m\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else if request.starts_with("GET /truncated") {
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        VIDEO.len() + 10
                    )
                    .unwrap();
                    socket.write_all(VIDEO).unwrap();
                } else {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Encoding: Identity\r\nConnection: close\r\n\r\n").unwrap();
                    write!(socket, "{:x}\r\n", VIDEO.len()).unwrap();
                    socket.write_all(VIDEO).unwrap();
                    socket.write_all(b"\r\n0\r\n\r\n").unwrap();
                }
            }
        });
        for path in ["/video.y4m", "/redirect"] {
            let input =
                DownloadedInput::download(&format!("http://{address}{path}"), &Default::default())
                    .unwrap();
            assert_eq!(
                crate::owned_y4m_decode::decode_video(input.path())
                    .unwrap()
                    .video_frames,
                3
            );
        }
        assert!(
            DownloadedInput::download(&format!("http://{address}/truncated"), &Default::default())
                .is_err()
        );
        server.join().unwrap();
    }
}
