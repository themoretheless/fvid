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
    source_url: Option<reqwest::Url>,
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
        let effective_url = response.url().clone();
        let length = response.content_length();
        let extension = Path::new(effective_url.path())
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("media");
        let extension =
            if extension.len() <= 16 && extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
                extension
            } else {
                "media"
            };
        let mut input = Self::from_body(response, length, extension, options)?;
        input.source_url = Some(effective_url);
        Ok(input)
    }
    /// Download finite media, resolving finite fMP4 HLS manifests when present.
    pub fn download_media(url: &str, options: &DownloadOptions) -> Result<std::sync::Arc<Self>> {
        use crate::owned_hls::{self, Playlist};
        use std::{
            collections::{HashMap, HashSet},
            sync::Arc,
        };
        let mut cache = HashMap::<String, Arc<Self>>::new();
        let mut downloaded = 0u64;
        let mut fetch = |url: &reqwest::Url| -> Result<Arc<Self>> {
            cancelled(options)?;
            if let Some(input) = cache.get(url.as_str()) {
                return Ok(input.clone());
            }
            let mut child = options.clone();
            child.max_bytes = options
                .max_bytes
                .map(|limit| limit.saturating_sub(downloaded));
            let before = downloaded;
            let progress = options.progress.clone();
            child.progress = progress.map(|progress| {
                ProgressHook::new(move |event| {
                    progress.emit(ProgressEvent {
                        payload_bytes: before.saturating_add(event.payload_bytes),
                        done: false,
                        ..event
                    });
                })
            });
            let input = Arc::new(Self::download(url.as_str(), &child)?);
            downloaded = downloaded
                .checked_add(input.bytes)
                .ok_or("HTTP aggregate byte count overflow")?;
            cache.insert(url.as_str().to_owned(), input.clone());
            Ok(input)
        };
        let mut current = reqwest::Url::parse(url).map_err(|_| "invalid HTTP media URL")?;
        let mut visited = HashSet::new();
        let result = loop {
            if !visited.insert(current.as_str().to_owned()) {
                return Err("cyclic HLS master playlist".into());
            }
            let input = fetch(&current)?;
            let mut file = File::open(input.path()).map_err(|e| e.to_string())?;
            let mut prefix = [0u8; 7];
            let count = file.read(&mut prefix).map_err(|e| e.to_string())?;
            if count != 7 || &prefix != b"#EXTM3U" {
                if visited.len() > 1 {
                    return Err("HLS variant response is not a playlist".into());
                }
                break input;
            }
            let text = std::fs::read_to_string(input.path())
                .map_err(|e| format!("HLS manifest text: {e}"))?;
            let base = input
                .source_url
                .as_ref()
                .ok_or("HLS input lacks effective response URL")?;
            match owned_hls::parse(&text)? {
                Playlist::Master(variants) => {
                    let variant = variants
                        .iter()
                        .max_by_key(|variant| variant.bandwidth)
                        .ok_or("empty HLS master")?;
                    if variant.external_renditions {
                        return Err("HLS external renditions are not yet supported".into());
                    }
                    current = base
                        .join(variant.uri)
                        .map_err(|_| "invalid HLS variant URL")?;
                }
                Playlist::Media(playlist) => {
                    let (mut output, mut file) = Self::create_file("mp4")?;
                    let assembled = owned_hls::assemble_fmp4(
                        &playlist,
                        &mut file,
                        |resource| {
                            let url = base
                                .join(resource.uri)
                                .map_err(|_| "invalid HLS resource URL")?;
                            let asset = fetch(&url)?;
                            Ok(Box::new(
                                File::open(asset.path()).map_err(|e| e.to_string())?,
                            ))
                        },
                        options.max_bytes,
                        options.cancel.as_ref(),
                    );
                    let flushed = file.flush().map_err(|e| e.to_string());
                    drop(file);
                    output.bytes = assembled?;
                    flushed?;
                    output.source_url = Some(base.clone());
                    break Arc::new(output);
                }
            }
        };
        // Release the borrowing fetch closure before reporting aggregate completion.
        drop(fetch);
        if let Some(progress) = &options.progress {
            progress.emit(ProgressEvent {
                packets: 0,
                payload_bytes: downloaded,
                done: true,
            });
        }
        Ok(result)
    }
    fn create_file(extension: &str) -> Result<(Self, File)> {
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
        let input = Self {
            path: directory.join(format!("source.{extension}")),
            directory,
            bytes: 0,
            source_url: None,
        };
        let file = File::create(&input.path).map_err(|e| e.to_string())?;
        Ok((input, file))
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
        let (mut input, mut file) = Self::create_file(extension)?;
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
        assert!(DownloadedInput::from_body(VIDEO, None, "y4m", &limited)
            .unwrap_err()
            .contains("byte limit"));
        let flag = CancelFlag::new();
        let trigger = flag.clone();
        let options = DownloadOptions {
            cancel: Some(flag),
            progress: Some(ProgressHook::new(move |_| trigger.cancel())),
            ..Default::default()
        };
        assert!(DownloadedInput::from_body(VIDEO, None, "y4m", &options)
            .unwrap_err()
            .contains("cancelled"));
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
        assert!(DownloadedInput::download(
            &format!("http://{address}/truncated"),
            &Default::default()
        )
        .is_err());
        server.join().unwrap();
    }
}

#[cfg(test)]
mod hls_tests {
    use super::*;
    #[test]
    #[ignore = "explicit loopback HTTP integration; ordinary tests do not use networking"]
    fn redirect_master_and_byte_ranges_open_owned_fragmented_media() {
        use std::{
            io::{BufRead, BufReader},
            net::TcpListener,
            thread,
        };
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/hls");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut requests = Vec::new();
            while requests.len() < 4 {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "server request timeout"
                        );
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                requests.push(path.clone());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                if path == "/start" {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /nested/master.m3u8\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let body = if path == "/nested/master.m3u8" {
                        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nbyterange.m3u8\n".to_vec()
                    } else {
                        std::fs::read(fixtures.join(path.strip_prefix("/nested/").unwrap()))
                            .unwrap()
                    };
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(&body).unwrap();
                }
            }
            requests
        });
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let report = events.clone();
        let options = DownloadOptions {
            progress: Some(ProgressHook::new(move |event| {
                report.lock().unwrap().push(event)
            })),
            ..Default::default()
        };
        let input =
            DownloadedInput::download_media(&format!("http://{address}/start"), &options).unwrap();
        let reader = crate::owned_mp4::Mp4Reader::open(
            File::open(input.path()).unwrap(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(reader.tracks()[0].samples.len(), 25);
        assert_eq!(
            server.join().unwrap(),
            [
                "/start",
                "/nested/master.m3u8",
                "/nested/byterange.m3u8",
                "/nested/objects.mp4"
            ]
        );
        let events = events.lock().unwrap();
        assert_eq!(events.iter().filter(|event| event.done).count(), 1);
        assert!(events
            .windows(2)
            .all(|pair| pair[0].payload_bytes <= pair[1].payload_bytes));
        let path = input.path().to_owned();
        drop(input);
        assert!(!path.exists());
    }
}
