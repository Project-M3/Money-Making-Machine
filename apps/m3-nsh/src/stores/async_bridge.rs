//! Event-driven Worker Thread -> NSH Signal async bridge (zero idle polling).
//!
//! Spawns plain OS worker threads (`m3-worker`, outside Tokio's `spawn_blocking`
//! context so `reqwest::blocking::Client` never panics on drop) and wakes the
//! NSH engine thread immediately via `oneshot` / `mpsc` channels.

use nsh_core::prelude::spawn;
use std::sync::Arc;
use tokio::sync::{Notify, mpsc, oneshot};

/// Runs a blocking closure on a plain OS worker thread and delivers the result back on the engine thread.
pub fn run_blocking_result<T, F, C>(work: F, on_done: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = oneshot::channel::<T>();
    if let Err(err) = std::thread::Builder::new()
        .name("m3-worker".into())
        .spawn(move || {
            let result = work();
            let _ = tx.send(result);
        })
    {
        log::warn!("m3-nsh failed to spawn worker thread: {err}");
    }
    spawn(async move {
        if let Ok(result) = rx.await {
            on_done(result);
        } else {
            log::warn!("m3-nsh blocking worker exited without delivering a result");
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
        .spawn(move || work(tx))
    {
        log::warn!("m3-nsh failed to spawn streaming worker thread: {err}");
    }
    spawn(async move {
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
    spawn(async move {
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
