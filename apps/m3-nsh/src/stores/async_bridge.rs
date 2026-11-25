//! Event-driven Worker Thread -> NSH Signal async bridge (zero idle polling).
//!
//! Spawns plain OS worker threads (`m3-worker`, outside Tokio's `spawn_blocking`
//! context so `reqwest::blocking::Client` never panics on drop) and wakes the
//! NSH engine thread immediately via `oneshot` / `mpsc` channels.

use nsh_core::prelude::{Owner, spawn};
use std::sync::Arc;
use tokio::sync::{Notify, mpsc, oneshot};

/// Spawns an engine-thread async task under `Owner::detached()` so re-rendering a
/// `dynamic(...)` / `when(...)` branch into a loading state or switching tabs never
/// cancels an in-flight background store task before its callback runs.
pub fn spawn_detached<F>(future: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    Owner::detached().run(move || {
        drop(spawn(future));
    });
}

fn format_panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        format!("worker panicked: {s}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("worker panicked: {s}")
    } else {
        "worker panicked".to_owned()
    }
}

/// Runs a blocking closure on a plain OS worker thread and delivers the result (or a worker/panic
/// error) back on the engine thread. Guaranteed to invoke `on_done` once even if spawning fails
/// or the worker panics.
pub fn run_blocking_or_err<T, F, C>(work: F, on_done: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(Result<T, String>) + 'static,
{
    let (tx, rx) = oneshot::channel::<Result<T, String>>();
    if let Err(err) = std::thread::Builder::new()
        .name("m3-worker".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .map_err(format_panic_payload);
            let _ = tx.send(outcome);
        })
    {
        log::warn!("m3-nsh failed to spawn worker thread: {err}");
        let msg = format!("failed to spawn worker thread: {err}");
        spawn_detached(async move {
            on_done(Err(msg));
        });
        return;
    }
    spawn_detached(async move {
        let outcome = match rx.await {
            Ok(res) => res,
            Err(_) => {
                log::warn!("m3-nsh blocking worker exited without delivering a result");
                Err("worker exited unexpectedly".to_owned())
            }
        };
        on_done(outcome);
    });
}

/// Runs a blocking closure on a plain OS worker thread and delivers the result back on the engine thread.
pub fn run_blocking_result<T, F, C>(work: F, on_done: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    run_blocking_or_err(work, move |outcome| {
        if let Ok(result) = outcome {
            on_done(result);
        }
    });
}

/// Runs a streaming blocking task using a plain OS worker thread and `unbounded_channel`.
///
/// Each event sent to `tx` immediately wakes the engine-thread `spawn` task via `rx.recv().await`.
pub fn run_streamed_blocking<E, F, C, D>(work: F, mut on_event: C, on_finished: D)
where
    E: Send + 'static,
    F: FnOnce(mpsc::UnboundedSender<E>) + Send + 'static,
    C: FnMut(E) + 'static,
    D: FnOnce() + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<E>();
    if let Err(err) = std::thread::Builder::new()
        .name("m3-worker".into())
        .spawn(move || {
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(tx)))
            {
                log::warn!("m3-nsh streamed {}", format_panic_payload(payload));
            }
        })
    {
        log::warn!("m3-nsh failed to spawn streaming worker thread: {err}");
    }
    spawn_detached(async move {
        while let Some(ev) = rx.recv().await {
            on_event(ev);
        }
        on_finished();
    });
}

/// Spawns an event-driven wakeup listener for [`m3_core::alan_interrogator::M3InterrogatorHandle`].
///
/// The task suspends at `notify.notified().await` (consuming 0 CPU/tokens) and wakes immediately
/// whenever `M3InterrogatorHandle` pushes an `InterrogatorEvent`.
pub fn spawn_notify_listener(notify: Arc<Notify>, mut on_wake: impl FnMut() + 'static) {
    spawn_detached(async move {
        loop {
            notify.notified().await;
            on_wake();
        }
    });
}

/// Enforces DuckDB lock discipline: no read-only open while an exclusive L0/L1 writer holds the DB lock.
pub fn can_open_duckdb_read_only(universe_syncing: bool, inbox_catching_up: bool) -> bool {
    !universe_syncing && !inbox_catching_up
}

/// Enforces DuckDB exclusive writer discipline across L0 Universe sync, L1 catch-up, and Circle sync.
pub fn can_start_duckdb_writer(
    universe_syncing: bool,
    inbox_catching_up: bool,
    sync_running: bool,
) -> bool {
    !universe_syncing && !inbox_catching_up && !sync_running
}
