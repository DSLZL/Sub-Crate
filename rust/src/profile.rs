//! Opt-in, worker-local pipeline timings. Never inspect user values for labels.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    future::{Future, poll_fn},
    io::Write,
    pin::pin,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

const LIMIT: usize = 2048;
type Key = (Option<usize>, &'static str, Option<usize>);

#[derive(Default)]
struct Row {
    calls: u64,
    wall_ns: u64,
    active_ns: u64,
    self_ns: u64,
    max_wall_ns: u64,
}

struct Frame {
    id: usize,
    began: Instant,
    children: u64,
}

struct State {
    job: u64,
    began: Instant,
    operation: &'static str,
    rows: Vec<(Key, Row)>,
    keys: HashMap<Key, usize>,
    stack: Vec<Frame>,
    sequence: usize,
    dropped: u64,
    unfinished: usize,
}

thread_local! {
    static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn nanos(began: Instant) -> u64 {
    began.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SUB_STORE_PROFILE").is_ok_and(|v| v == "1"))
}

pub(crate) async fn request<T>(future: impl Future<Output = T>) -> T {
    let began = enabled().then(Instant::now);
    let result = future.await;
    if let Some(began) = began {
        let report = json!({"version":1, "wall_ns":nanos(began)});
        let _ = writeln!(std::io::stdout().lock(), "SUB_STORE_HTTP_PROFILE {report}");
    }
    result
}

pub(crate) struct Session;

impl Session {
    pub(crate) fn start(operation: &str, queued: Instant) -> Self {
        let session = Self::new(enabled(), operation);
        CURRENT.with_borrow_mut(|current| {
            if let Some(state) = current {
                state.began = queued;
                let wall_ns = nanos(queued);
                let key = (None, "worker_queue", None);
                state.keys.insert(key, 0);
                state.rows.push((
                    key,
                    Row {
                        calls: 1,
                        wall_ns,
                        max_wall_ns: wall_ns,
                        ..Row::default()
                    },
                ));
            }
        });
        session
    }

    fn new(enabled: bool, operation: &str) -> Self {
        static JOB: AtomicU64 = AtomicU64::new(1);
        if enabled {
            let operation = match operation {
                "download-sub" => "download_sub",
                "download-collection" => "download_collection",
                "download-file" => "download_file",
                _ => "other",
            };
            CURRENT.with_borrow_mut(|current| {
                assert!(current.is_none(), "Nested profile session");
                *current = Some(State {
                    job: JOB.fetch_add(1, Ordering::Relaxed),
                    began: Instant::now(),
                    operation,
                    rows: Vec::new(),
                    keys: HashMap::new(),
                    stack: Vec::new(),
                    sequence: 0,
                    dropped: 0,
                    unfinished: 0,
                });
            });
        }
        Self
    }

    fn report(&self, success: bool) -> Option<Value> {
        CURRENT.with_borrow_mut(|current| {
            let state = current.take()?;
            let rows: Vec<_> = state.rows.into_iter().enumerate().map(|(id, ((parent, stage, index), row))| json!({
                "id": id, "parent": parent, "stage": stage, "index": index,
                "calls": row.calls, "wall_ns": row.wall_ns, "active_ns": row.active_ns,
                "self_ns": row.self_ns, "max_wall_ns": row.max_wall_ns,
            })).collect();
            Some(json!({"version": 1, "job": state.job, "operation": state.operation,
                "success": success, "wall_ns": nanos(state.began), "dropped": state.dropped,
                "open_scopes": state.stack.len(), "unfinished_spans": state.unfinished, "rows": rows}))
        })
    }

    pub(crate) fn finish(&self, success: bool) {
        if let Some(report) = self.report(success) {
            // A closed pipe must not change the script result.
            let _ = writeln!(std::io::stdout().lock(), "SUB_STORE_PROFILE {report}");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|current| {
            current.take();
        });
    }
}

struct Span {
    job: u64,
    id: usize,
    began: Instant,
}

impl Span {
    fn new(stage: &'static str, index: Option<usize>) -> Option<Self> {
        CURRENT.with_borrow_mut(|current| {
            let state = current.as_mut()?;
            let key = (state.stack.last().map(|frame| frame.id), stage, index);
            let id = if let Some(id) = state.keys.get(&key) {
                *id
            } else {
                if state.rows.len() == LIMIT {
                    state.dropped += 1;
                    return None;
                }
                let id = state.rows.len();
                state.keys.insert(key, id);
                state.rows.push((key, Row::default()));
                id
            };
            state.unfinished += 1;
            Some(Self {
                job: state.job,
                id,
                began: Instant::now(),
            })
        })
    }

    fn enter(&self) -> Active {
        CURRENT.with_borrow_mut(|current| {
            if let Some(state) = current.as_mut().filter(|state| state.job == self.job) {
                state.stack.push(Frame {
                    id: self.id,
                    began: Instant::now(),
                    children: 0,
                });
            }
        });
        Active {
            job: self.job,
            id: self.id,
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|current| {
            if let Some(state) = current.as_mut().filter(|state| state.job == self.job) {
                state.unfinished -= 1;
                let elapsed = nanos(self.began);
                let row = &mut state.rows[self.id].1;
                row.calls += 1;
                row.wall_ns += elapsed;
                row.max_wall_ns = row.max_wall_ns.max(elapsed);
            }
        });
    }
}

struct Active {
    job: u64,
    id: usize,
}

impl Drop for Active {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|current| {
            if let Some(state) = current.as_mut().filter(|state| state.job == self.job) {
                let frame = state.stack.pop().expect("Profile scope stack");
                assert_eq!(frame.id, self.id);
                let elapsed = nanos(frame.began);
                let row = &mut state.rows[self.id].1;
                row.active_ns += elapsed;
                row.self_ns += elapsed.saturating_sub(frame.children);
                if let Some(parent) = state.stack.last_mut() {
                    parent.children += elapsed;
                }
            }
        });
    }
}

pub(crate) struct Scope {
    // Drop the active interval before finishing its wall-clock span.
    _active: Option<Active>,
    _span: Option<Span>,
}

pub(crate) fn span(stage: &'static str) -> Scope {
    let span = Span::new(stage, None);
    Scope {
        _active: span.as_ref().map(Span::enter),
        _span: span,
    }
}

pub(crate) fn sequence() -> Option<usize> {
    CURRENT.with_borrow_mut(|current| {
        let state = current.as_mut()?;
        let index = state.sequence;
        state.sequence += 1;
        Some(index)
    })
}

/// Re-enter on each poll: an async sibling never inherits a suspended processor.
/// Active time is elapsed time in polls, not operating-system CPU time.
pub(crate) async fn measure<T>(
    stage: &'static str,
    index: Option<usize>,
    future: impl Future<Output = T>,
) -> T {
    let Some(span) = Span::new(stage, index) else {
        return future.await;
    };
    let mut future = pin!(future);
    poll_fn(|context| {
        let _active = span.enter();
        future.as_mut().poll(context)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn async_siblings_keep_their_own_children_and_partition_active_time() {
        let session = Session::new(true, "download-sub");
        measure("job", None, async {
            let child = |index| {
                measure("processor", Some(index), async move {
                    tokio::task::yield_now().await;
                    let _span = span("produce");
                    std::hint::black_box(index);
                })
            };
            tokio::join!(child(0), child(1));
        })
        .await;
        let report = session.report(true).unwrap();
        let rows = report["rows"].as_array().unwrap();
        let root = &rows[0];
        let processors: Vec<_> = rows
            .iter()
            .filter(|row| row["stage"] == "processor")
            .collect();
        assert_eq!(processors.len(), 2);
        for processor in processors {
            assert_eq!(processor["parent"], root["id"]);
            let children: Vec<_> = rows
                .iter()
                .filter(|row| row["parent"] == processor["id"])
                .collect();
            assert_eq!(children.len(), 1);
            assert_eq!(children[0]["stage"], "produce");
            assert_eq!(children[0]["calls"], 1);
        }
        let self_ns: u64 = rows
            .iter()
            .map(|row| row["self_ns"].as_u64().unwrap())
            .sum();
        assert_eq!(self_ns, root["active_ns"].as_u64().unwrap());
        assert!(root["wall_ns"].as_u64().unwrap() >= root["active_ns"].as_u64().unwrap());
        assert_eq!(report["open_scopes"], 0);
        assert_eq!(report["dropped"], 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disabled_and_failed_calls_preserve_results_and_reset_the_worker() {
        let off = Session::new(false, "private-sentinel");
        let result: Result<(), &str> =
            measure("job", None, async { Err("original failure") }).await;
        assert_eq!(result, Err("original failure"));
        assert!(off.report(false).is_none());
        drop(off);
        let on = Session::new(true, "private-sentinel");
        let result: Result<(), &str> = measure("job", None, async {
            let _span = span("produce");
            Err("original failure")
        })
        .await;
        assert_eq!(result, Err("original failure"));
        let report = on.report(false).unwrap();
        assert_eq!(report["success"], false);
        assert_eq!(report["open_scopes"], 0);
        assert_eq!(report["rows"][1]["calls"], 1);
        assert!(!report.to_string().contains("private-sentinel"));
        drop(on);
        let next = Session::new(true, "download-sub");
        assert_eq!(next.report(true).unwrap()["rows"], json!([]));
    }

    #[test]
    fn aggregation_is_bounded_and_cancellation_closes_the_span() {
        let session = Session::new(true, "download-sub");
        for _ in 0..3 {
            let _span = span("produce");
        }
        let mut cancelled = Box::pin(measure("script_await", None, std::future::pending::<()>()));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(cancelled.as_mut().poll(&mut context).is_pending());
        drop(cancelled);
        for index in 0..LIMIT {
            drop(Span::new("processor", Some(index)));
        }
        let report = session.report(false).unwrap();
        assert_eq!(report["rows"].as_array().unwrap().len(), LIMIT);
        assert_eq!(report["rows"][0]["calls"], 3);
        assert_eq!(report["rows"][1]["calls"], 1);
        assert_eq!(report["dropped"], 2);
        assert_eq!(report["open_scopes"], 0);
        assert_eq!(report["unfinished_spans"], 0);
    }
}
