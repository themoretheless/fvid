//! Persistent elastic scoped worker pool and cached CPU parallelism policy.
//!
//! Spawning and tearing down OS threads via `std::thread::scope` on every video
//! frame costs tens of microseconds and hundreds of kernel transitions per
//! second at 60 fps. This module keeps an elastic freelist of parked worker
//! threads that wake on private condition variables, while preserving the exact
//! scoped borrow and panic-propagation guarantees of `std::thread::scope`.

use std::any::Any;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Cached hardware parallelism (`>= 1`), queried from the OS at most once.
#[inline]
pub fn parallelism() -> usize {
    static PARALLELISM: OnceLock<usize> = OnceLock::new();
    *PARALLELISM.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// How many workers a packed frame of `bytes` bytes should be split across.
///
/// Each span must carry enough pixels (`>= 16_384`) to amortize waking a
/// worker, keeping small frames on the calling thread.
#[inline]
pub fn span_workers(bytes: usize) -> usize {
    const MIN_SPAN_PIXELS: usize = 1 << 14;
    parallelism().min(bytes / 3 / MIN_SPAN_PIXELS).max(1)
}

type BoxedJob = Box<dyn FnOnce() + Send + 'static>;

struct WorkerSlot {
    job: Mutex<Option<BoxedJob>>,
    wake: Condvar,
}

impl WorkerSlot {
    fn new(initial: BoxedJob) -> Arc<Self> {
        Arc::new(Self {
            job: Mutex::new(Some(initial)),
            wake: Condvar::new(),
        })
    }
}

fn idle_pool() -> &'static Mutex<Vec<Arc<WorkerSlot>>> {
    static POOL: OnceLock<Mutex<Vec<Arc<WorkerSlot>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(Vec::new()))
}

fn dispatch_job(job: BoxedJob) {
    let reused = {
        let mut idle = idle_pool().lock().unwrap_or_else(|e| e.into_inner());
        idle.pop()
    };
    if let Some(slot) = reused {
        let mut guard = slot.job.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(job);
        drop(guard);
        slot.wake.notify_one();
        return;
    }

    let slot = WorkerSlot::new(job);
    let worker_slot = Arc::clone(&slot);
    let max_idle = (parallelism().saturating_mul(2)).max(8);
    let spawn_res = std::thread::Builder::new()
        .name("fvid-worker".into())
        .spawn(move || worker_loop(worker_slot, max_idle));

    if spawn_res.is_err() {
        // Fallback: execute synchronously on the caller if the OS refuses a new thread.
        let inline_job = slot
            .job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(f) = inline_job {
            f();
        }
    }
}

fn worker_loop(slot: Arc<WorkerSlot>, max_idle: usize) {
    loop {
        let next_job = {
            let mut guard = slot.job.lock().unwrap_or_else(|e| e.into_inner());
            while guard.is_none() {
                guard = slot.wake.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
            guard.take()
        };
        if let Some(job) = next_job {
            job();
        }
        let mut idle = idle_pool().lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < max_idle {
            idle.push(Arc::clone(&slot));
        } else {
            return;
        }
    }
}

struct TaskOutcome<T> {
    finished: bool,
    joined: bool,
    result: Option<std::thread::Result<T>>,
}

struct TaskState<T> {
    state: Mutex<TaskOutcome<T>>,
    done: Condvar,
}

trait ScopeTask: Send + Sync {
    fn wait_and_cleanup(&self, unhandled_panic: &mut Option<Box<dyn Any + Send + 'static>>);
}

impl<T: Send> ScopeTask for TaskState<T> {
    fn wait_and_cleanup(&self, unhandled_panic: &mut Option<Box<dyn Any + Send + 'static>>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while !guard.finished {
            guard = self.done.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        // Always drop any unjoined result before `'env` ends; if an unjoined task
        // panicked, record the first panic payload to re-raise after all tasks finish.
        if let Some(res) = guard.result.take()
            && !guard.joined
            && let Err(payload) = res
            && unhandled_panic.is_none()
        {
            *unhandled_panic = Some(payload);
        }
    }
}

/// Handle to a task running on the persistent scoped worker pool.
pub struct ScopedJoinHandle<'scope, T> {
    task: Arc<TaskState<T>>,
    _marker: PhantomData<&'scope mut &'scope ()>,
}

impl<T> ScopedJoinHandle<'_, T> {
    /// Wait for the task to complete and return its result or panic payload.
    pub fn join(self) -> std::thread::Result<T> {
        let mut guard = self.task.state.lock().unwrap_or_else(|e| e.into_inner());
        while !guard.finished {
            guard = self.task.done.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        guard.joined = true;
        guard
            .result
            .take()
            .expect("scoped task finished without storing outcome")
    }
}

/// Scoped spawner backed by the persistent `fvid-cpu` worker pool.
pub struct Scope<'scope, 'env: 'scope> {
    tasks: Mutex<Vec<Arc<dyn ScopeTask + 'static>>>,
    _scope: PhantomData<&'scope mut &'scope ()>,
    _env: PhantomData<&'env mut &'env ()>,
}

impl<'scope, 'env> Scope<'scope, 'env> {
    /// Spawn a scoped closure onto an idle worker thread (or create one on cold start).
    pub fn spawn<F, T>(&'scope self, f: F) -> ScopedJoinHandle<'scope, T>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        let task = Arc::new(TaskState {
            state: Mutex::new(TaskOutcome {
                finished: false,
                joined: false,
                result: None,
            }),
            done: Condvar::new(),
        });

        // SAFETY: `ScopeGuard::drop` unconditionally blocks until every spawned
        // task sets `finished = true` AND drops any unjoined `T` before `scope()`
        // returns or unwinds, so neither `F` nor `T` can outlive `'scope` / `'env`.
        let task_erased: Arc<dyn ScopeTask + 'scope> = Arc::clone(&task) as Arc<dyn ScopeTask + 'scope>;
        let task_static: Arc<dyn ScopeTask + 'static> =
            unsafe { std::mem::transmute(task_erased) };
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(task_static);

        let worker_task = Arc::clone(&task);
        let boxed_scope: Box<dyn FnOnce() + Send + 'scope> = Box::new(move || {
            let res = catch_unwind(AssertUnwindSafe(f));
            let mut guard = worker_task.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.result = Some(res);
            guard.finished = true;
            drop(guard);
            worker_task.done.notify_all();
        });
        // SAFETY: Same lifetime guarantee enforced by `ScopeGuard::drop` above.
        let boxed_static: BoxedJob = unsafe { std::mem::transmute(boxed_scope) };
        dispatch_job(boxed_static);

        ScopedJoinHandle {
            task,
            _marker: PhantomData,
        }
    }
}

/// Execute `f` with a scoped spawner backed by the persistent `fvid-cpu` pool.
///
/// All tasks spawned inside `f` are guaranteed to complete before `scope`
/// returns, even if `f` or any worker panics.
pub fn scope<'env, F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
{
    let scope_data = Scope {
        tasks: Mutex::new(Vec::new()),
        _scope: PhantomData,
        _env: PhantomData,
    };

    // SAFETY: `f` accepts `&'scope Scope<'scope, 'env>` for ANY `'scope` chosen
    // by `scope()`, and we unconditionally drain and wait for every task in
    // `scope_data.tasks` below before `scope_data` (and `'env`) is dropped.
    let scope_ref: &Scope<'_, 'env> = unsafe { std::mem::transmute(&scope_data) };
    let body_res = catch_unwind(AssertUnwindSafe(|| f(scope_ref)));

    let mut unhandled_panic: Option<Box<dyn Any + Send + 'static>> = None;
    loop {
        let batch: Vec<Arc<dyn ScopeTask + 'static>> = {
            let mut tasks = scope_data.tasks.lock().unwrap_or_else(|e| e.into_inner());
            if tasks.is_empty() {
                break;
            }
            std::mem::take(&mut *tasks)
        };
        for task in batch {
            task.wait_and_cleanup(&mut unhandled_panic);
        }
    }

    match body_res {
        Ok(val) => {
            if let Some(payload) = unhandled_panic {
                resume_unwind(payload);
            }
            val
        }
        Err(payload) => resume_unwind(payload),
    }
}

/// Process mutable `slice` in `chunk_len`-sized chunks across the persistent
/// worker pool, running chunk `0` inline on the calling thread to avoid waking
/// an extra worker unnecessarily.
pub fn par_chunks_mut<T, F>(slice: &mut [T], chunk_len: usize, f: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    assert!(chunk_len > 0, "chunk_len must be non-zero");
    if slice.len() <= chunk_len {
        if !slice.is_empty() {
            f(0, slice);
        }
        return;
    }
    let f_ref = &f;
    scope(|s| {
        let mut iter = slice.chunks_mut(chunk_len).enumerate();
        let first = iter.next();
        for (idx, chunk) in iter {
            s.spawn(move || f_ref(idx, chunk));
        }
        if let Some((idx, chunk)) = first {
            f_ref(idx, chunk);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_borrows_stack_and_reuses_workers() {
        let weights = [3i32, 5, 7, 11];
        let mut data = vec![1i32; 4096];
        for _ in 0..8 {
            par_chunks_mut(&mut data, 512, |chunk_idx, chunk| {
                let w = weights[chunk_idx % weights.len()];
                for v in chunk {
                    *v += w;
                }
            });
        }
        assert_eq!(data[0], 1 + 8 * 3);
        assert_eq!(data[512], 1 + 8 * 5);
        assert_eq!(data[1024], 1 + 8 * 7);
        assert_eq!(data[1536], 1 + 8 * 11);
    }

    #[test]
    fn join_handle_returns_value_and_captures_panic() {
        let base = 40;
        let out = scope(|s| {
            let h1 = s.spawn(|| base + 2);
            let h2 = s.spawn(|| -> i32 { panic!("boom") });
            assert_eq!(h1.join().unwrap(), 42);
            assert!(h2.join().is_err());
            99
        });
        assert_eq!(out, 99);
    }

    #[test]
    fn unjoined_panic_propagates_after_all_workers_complete() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = AtomicUsize::new(0);
        let res = catch_unwind(AssertUnwindSafe(|| {
            scope(|s| {
                s.spawn(|| {
                    counter.fetch_add(1, Ordering::SeqCst);
                });
                s.spawn(|| {
                    panic!("unjoined worker panic");
                });
            });
        }));
        assert!(res.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn nested_scopes_do_not_deadlock() {
        let mut grid = vec![0u32; 256];
        scope(|outer| {
            for (row, slice) in grid.chunks_mut(64).enumerate() {
                outer.spawn(move || {
                    par_chunks_mut(slice, 16, |col, sub| {
                        for x in sub {
                            *x = (row * 10 + col) as u32;
                        }
                    });
                });
            }
        });
        assert_eq!(grid[0], 0);
        assert_eq!(grid[16], 1);
        assert_eq!(grid[64], 10);
        assert_eq!(grid[240], 33);
    }
}
