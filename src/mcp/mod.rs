//! MCP adapters. Native media operations run directly on bounded blocking workers.
mod tools;
mod transport;
use rmcp::{ErrorData, RoleServer, ServerHandler, model::*, service::RequestContext};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Server {
    root: Arc<PathBuf>,
    gate: Arc<Semaphore>,
}
impl Server {
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::with_jobs(root, 1)
    }
    /// Bound independent operations across all clones and HTTP sessions.
    pub fn with_jobs(root: impl AsRef<Path>, jobs: usize) -> std::io::Result<Self> {
        if !(1..=32).contains(&jobs) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "jobs must be between 1 and 32",
            ));
        }
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::other("MCP root must be a directory"));
        }
        Ok(Self {
            root: Arc::new(root),
            gate: Arc::new(Semaphore::new(jobs)),
        })
    }
    fn start_job<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<tokio::task::JoinHandle<T>, tokio::sync::TryAcquireError> {
        let permit = self.gate.clone().try_acquire_owned()?;
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        }))
    }
    fn input(&self, path: &str) -> Result<PathBuf, String> {
        let resolved = self
            .root
            .join(path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !resolved.starts_with(self.root.as_path()) || !resolved.is_file() {
            return Err("input must be a regular file within MCP root".into());
        }
        Ok(resolved)
    }
    fn output(&self, path: &str) -> Result<PathBuf, String> {
        let requested = self.root.join(path);
        let filename = requested.file_name().ok_or("output filename required")?;
        let parent = requested
            .parent()
            .ok_or("output directory required")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !parent.starts_with(self.root.as_path()) {
            return Err("output must be within MCP root".into());
        }
        let resolved = parent.join(filename);
        if resolved.symlink_metadata().is_ok() {
            return Err("output already exists".into());
        }
        Ok(resolved)
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fvid", env!("CARGO_PKG_VERSION")))
            .with_instructions("Fvid edits local media within its configured root. Outputs must be new files. Native codecs run on CPU; GPU processing is exposed by fvid_process_y4m. Lossless guarantees apply only to documented operations. Concurrent operations are bounded by the server jobs setting; excess calls return busy. Cancellation does not stop an already running native codec. File paths refer to the server host.")
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools::catalog().to_vec(),
            ..Default::default()
        })
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools::catalog().iter().find(|t| t.name == name).cloned()
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !tools::catalog().iter().any(|t| t.name == request.name) {
            return Err(ErrorData::invalid_params("unknown Fvid tool", None));
        }
        let server = self.clone();
        let job = match self.start_job(move || {
            crate::media::with_standalone_inputs(|| {
                tools::execute(
                    &server,
                    &request.name,
                    request.arguments.unwrap_or_default(),
                )
            })
        }) {
            Ok(job) => job,
            Err(_) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Fvid is busy; retry after an active operation finishes",
                )])
                .into());
            }
        };
        let result = job
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
        }
        .into())
    }
}
pub use transport::run;

#[cfg(test)]
mod allocation_tests {
    use super::*;
    #[test]
    fn tools_share_immutable_schemas_across_servers_and_threads() {
        let server = Server::new(".").unwrap();
        let tool = server.get_tool("fvid_process_y4m").unwrap();
        let other = std::thread::spawn(|| {
            Server::new(".")
                .unwrap()
                .get_tool("fvid_process_y4m")
                .unwrap()
        })
        .join()
        .unwrap();
        assert!(Arc::ptr_eq(&tool.input_schema, &other.input_schema));
        assert_eq!(tools::catalog().len(), 13);
        assert!(server.get_tool("missing").is_none());
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn jobs_overlap_and_cancelled_handles_keep_capacity_until_work_finishes() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let server = Server::with_jobs(".", 2).unwrap();
            let clone = server.clone();
            let (started, starts) = mpsc::channel();
            let (release_one, wait_one) = mpsc::channel();
            let (release_two, wait_two) = mpsc::channel();
            let first_started = started.clone();
            let first = server
                .start_job(move || {
                    first_started.send(1).unwrap();
                    wait_one.recv_timeout(Duration::from_secs(5)).unwrap();
                })
                .unwrap();
            let second = clone
                .start_job(move || {
                    started.send(2).unwrap();
                    wait_two.recv_timeout(Duration::from_secs(5)).unwrap();
                })
                .unwrap();
            // Both workers must start before either is allowed to finish.
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(server.start_job(|| ()).is_err());
            first.abort();
            assert!(clone.start_job(|| ()).is_err());
            // Async tasks remain schedulable while both native workers are blocked.
            assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
            release_one.send(()).unwrap();
            first.await.unwrap();
            assert_eq!(clone.start_job(|| 7).unwrap().await.unwrap(), 7);
            release_two.send(()).unwrap();
            second.await.unwrap();
            assert_eq!(server.gate.available_permits(), 2);
            assert!(
                server
                    .start_job(|| panic!("test worker failure"))
                    .unwrap()
                    .await
                    .is_err()
            );
            assert_eq!(server.gate.available_permits(), 2);
        });
    }
    #[test]
    fn job_limits_are_validated() {
        assert!(Server::with_jobs(".", 0).is_err());
        assert!(Server::with_jobs(".", 33).is_err());
    }
}
