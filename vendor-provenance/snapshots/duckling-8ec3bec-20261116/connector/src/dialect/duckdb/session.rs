use std::future::pending;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::dialect::duckdb::duckdb_sync::DuckDbSyncConnection;
use crate::error::ConnectorError;
use tokio::sync::{Mutex as AsyncMutex, watch};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSessionLifecycle {
  New,
  Opening,
  Ready,
  Retiring,
  Closed,
  Failed,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeSessionSnapshot {
  pub generation: u64,
  pub opens: usize,
  pub admitted: usize,
  pub active_operation: Option<u64>,
  pub last_completed_at: Instant,
  pub lifecycle: NativeSessionLifecycle,
}

struct ActiveOperation {
  generation: u64,
  id: u64,
  interrupt: Arc<duckdb::InterruptHandle>,
}

struct ControlState {
  lifecycle: NativeSessionLifecycle,
  admitted: usize,
  active: Option<ActiveOperation>,
  last_completed_at: Instant,
}

pub struct NativeSession {
  path: Option<String>,
  cwd: Option<String>,
  generation: u64,
  next_operation: AtomicU64,
  opens: AtomicUsize,
  execution: Arc<AsyncMutex<()>>,
  native: Mutex<Option<DuckDbSyncConnection>>,
  control: Mutex<ControlState>,
  retirement: watch::Sender<bool>,
  activity: watch::Sender<usize>,
  retirement_observers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
  #[cfg(test)]
  open_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for NativeSession {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let snapshot = self.snapshot();
    formatter
      .debug_struct("NativeSession")
      .field("generation", &snapshot.generation)
      .field("lifecycle", &snapshot.lifecycle)
      .field("admitted", &snapshot.admitted)
      .field("active_operation", &snapshot.active_operation)
      .finish()
  }
}

impl Default for NativeSession {
  fn default() -> Self {
    Self::new_local(None, None)
  }
}

impl NativeSession {
  pub fn new_local(path: Option<String>, cwd: Option<String>) -> Self {
    let (retirement, _) = watch::channel(false);
    let (activity, _) = watch::channel(0);
    Self {
      path,
      cwd,
      generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
      next_operation: AtomicU64::new(1),
      opens: AtomicUsize::new(0),
      execution: Arc::new(AsyncMutex::new(())),
      native: Mutex::new(None),
      control: Mutex::new(ControlState {
        lifecycle: NativeSessionLifecycle::New,
        admitted: 0,
        active: None,
        last_completed_at: Instant::now(),
      }),
      retirement,
      activity,
      retirement_observers: Mutex::new(Vec::new()),
      #[cfg(test)]
      open_hook: Mutex::new(None),
    }
  }

  pub fn snapshot(&self) -> NativeSessionSnapshot {
    match self.control.lock() {
      Ok(control) => NativeSessionSnapshot {
        generation: self.generation,
        opens: self.opens.load(Ordering::Relaxed),
        admitted: control.admitted,
        active_operation: control.active.as_ref().map(|active| active.id),
        last_completed_at: control.last_completed_at,
        lifecycle: control.lifecycle,
      },
      Err(_) => NativeSessionSnapshot {
        generation: self.generation,
        opens: self.opens.load(Ordering::Relaxed),
        admitted: 0,
        active_operation: None,
        last_completed_at: Instant::now(),
        lifecycle: NativeSessionLifecycle::Failed,
      },
    }
  }

  pub fn on_retire(&self, observer: impl FnOnce() + Send + 'static) {
    let mut observers = self.retirement_observers.lock().unwrap_or_else(|error| error.into_inner());
    if self.is_usable() { observers.push(Box::new(observer)); }
    else { drop(observers); observer(); }
  }

  fn notify_retired(&self) {
    let observers = std::mem::take(&mut *self.retirement_observers.lock().unwrap_or_else(|error| error.into_inner()));
    for observer in observers { let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(observer)); }
  }

  pub fn is_usable(&self) -> bool {
    matches!(
      self.snapshot().lifecycle,
      NativeSessionLifecycle::New | NativeSessionLifecycle::Opening | NativeSessionLifecycle::Ready
    )
  }

  pub async fn run<T, F>(
    self: &Arc<Self>,
    cancel: Option<&CancelToken>,
    work: F,
  ) -> anyhow::Result<T>
  where
    T: Send + 'static,
    F: FnOnce(&DuckDbSyncConnection, &CancelToken) -> anyhow::Result<T> + Send + 'static,
  {
    let activity = self.admit()?;
    let operation_id = self.next_operation.fetch_add(1, Ordering::Relaxed);
    let operation_token = CancelToken::new();
    let mut drop_guard = CallerDropGuard {
      token: operation_token.clone(),
      session: Arc::clone(self),
      generation: self.generation,
      operation_id,
      armed: true,
    };
    let mut retirement = self.retirement.subscribe();
    let permit = tokio::select! {
      biased;
      _ = operation_token.cancelled() => return Err(cancelled_error()),
      _ = wait_for_cancel(cancel.cloned()) => {
        operation_token.cancel();
        return Err(cancelled_error());
      }
      _ = retirement.changed() => {
        operation_token.cancel();
        return Err(cancelled_error());
      }
      permit = self.execution.clone().lock_owned() => permit,
    };

    if operation_token.is_cancelled()
      || cancel.is_some_and(CancelToken::is_cancelled)
      || self.is_retiring()
    {
      operation_token.cancel();
      return Err(cancelled_error());
    }

    let generation = self.generation;
    let session = Arc::clone(self);
    let worker_token = operation_token.clone();
    let worker_external = cancel.cloned();
    let join = tokio::task::spawn_blocking(move || {
      let _permit = permit;
      let _activity = activity;
      let _monitor = OperationMonitor::start(
        Arc::clone(&session), generation, operation_id, worker_token.clone(), worker_external.clone(),
      )?;
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.execute(
          generation,
          operation_id,
          &worker_token,
          worker_external.as_ref(),
          work,
        )
      }));
      let result = match result {
        Ok(result) => result,
        Err(_) => {
          session.mark_failed();
          Err(anyhow::anyhow!("native session worker panicked"))
        }
      };
      result
    });

    let result = match join.await {
      Ok(result) => result,
      Err(error) => {
        self.mark_failed();
        Err(anyhow::anyhow!("native session worker failed: {error}"))
      }
    };
    drop_guard.armed = false;
    result
  }

  pub async fn retire(self: &Arc<Self>) {
    self.mark_retiring();
    self.close_after_drain().await;
  }

  pub async fn retire_if_idle(self: &Arc<Self>, ttl: Duration, now: Instant) -> bool {
    if ttl.is_zero() {
      return false;
    }
    let marked = match self.control.lock() {
      Ok(mut control) => {
        if control.admitted != 0
          || now.saturating_duration_since(control.last_completed_at) < ttl
          || matches!(
            control.lifecycle,
            NativeSessionLifecycle::Retiring | NativeSessionLifecycle::Closed
          )
        {
          false
        } else {
          control.lifecycle = NativeSessionLifecycle::Retiring;
          true
        }
      }
      Err(_) => false,
    };
    if marked {
      self.retirement.send_replace(true);
      self.notify_retired();
      self.close_after_drain().await;
    }
    marked
  }

  fn admit(self: &Arc<Self>) -> anyhow::Result<ActivityLease> {
    let mut control = self
      .control
      .lock()
      .map_err(|_| anyhow::anyhow!("native session control lock poisoned"))?;
    if matches!(
      control.lifecycle,
      NativeSessionLifecycle::Retiring
        | NativeSessionLifecycle::Closed
        | NativeSessionLifecycle::Failed
    ) {
      return Err(anyhow::anyhow!(
        "native session is not accepting operations"
      ));
    }
    control.admitted += 1;
    self.activity.send_replace(control.admitted);
    Ok(ActivityLease {
      session: Arc::clone(self),
    })
  }

  fn execute<T, F>(
    &self,
    generation: u64,
    operation_id: u64,
    token: &CancelToken,
    external: Option<&CancelToken>,
    work: F,
  ) -> anyhow::Result<T>
  where
    F: FnOnce(&DuckDbSyncConnection, &CancelToken) -> anyhow::Result<T>,
  {
    if self.cancellation_requested(token, external) {
      return Err(cancelled_error());
    }

    let opened = self
      .native
      .lock()
      .map_err(|_| anyhow::anyhow!("native session connection lock poisoned"))?
      .is_some();
    if !opened {
      {
        let mut control = self
          .control
          .lock()
          .map_err(|_| anyhow::anyhow!("native session control lock poisoned"))?;
        if control.lifecycle == NativeSessionLifecycle::Retiring
          || self.cancellation_requested(token, external)
        {
          return Err(cancelled_error());
        }
        control.lifecycle = NativeSessionLifecycle::Opening;
      }

      #[cfg(test)]
      let open_hook = self.open_hook.lock().ok().and_then(|hook| hook.clone());
      #[cfg(test)]
      if let Some(open_hook) = open_hook {
        open_hook();
      }
      let native = match DuckDbSyncConnection::new_local(self.path.clone(), self.cwd.clone()) {
        Ok(native) => native,
        Err(error) => {
          if let Ok(mut control) = self.control.lock()
            && control.lifecycle != NativeSessionLifecycle::Retiring
          {
            control.lifecycle = NativeSessionLifecycle::Failed;
          }
          return Err(error);
        }
      };

      if self.cancellation_requested(token, external) {
        self.reset_opening();
        drop(native);
        return Err(cancelled_error());
      }
      let mut slot = self
        .native
        .lock()
        .map_err(|_| anyhow::anyhow!("native session connection lock poisoned"))?;
      if self.is_retiring() || self.cancellation_requested(token, external) {
        drop(slot);
        drop(native);
        return Err(cancelled_error());
      }
      *slot = Some(native);
      self.opens.fetch_add(1, Ordering::Relaxed);
      if let Ok(mut control) = self.control.lock() {
        control.lifecycle = NativeSessionLifecycle::Ready;
      }
    }

    let slot = self
      .native
      .lock()
      .map_err(|_| anyhow::anyhow!("native session connection lock poisoned"))?;
    let native = slot
      .as_ref()
      .ok_or_else(|| anyhow::anyhow!("native session connection is unavailable"))?;
    let interrupt = native.inner.interrupt_handle();
    {
      let mut control = self
        .control
        .lock()
        .map_err(|_| anyhow::anyhow!("native session control lock poisoned"))?;
      if control.lifecycle == NativeSessionLifecycle::Retiring
        || self.cancellation_requested(token, external)
      {
        return Err(cancelled_error());
      }
      control.lifecycle = NativeSessionLifecycle::Ready;
      control.active = Some(ActiveOperation {
        generation,
        id: operation_id,
        interrupt,
      });
    }

    let result = if self.cancellation_requested(token, external) {
      Err(cancelled_error())
    } else {
      work(native, token).map_err(|error| native.explain_remote_error(error))
    };
    let mut control = self
      .control
      .lock()
      .map_err(|_| anyhow::anyhow!("native session control lock poisoned"))?;
    let cancelled = self.cancellation_requested(token, external)
      || control.lifecycle == NativeSessionLifecycle::Retiring;
    if control
      .active
      .as_ref()
      .is_some_and(|active| active.generation == generation && active.id == operation_id)
    {
      control.active = None;
    }
    if control.lifecycle != NativeSessionLifecycle::Retiring {
      control.lifecycle = NativeSessionLifecycle::Ready;
    }
    if cancelled {
      Err(cancelled_error())
    } else {
      result
    }
  }

  fn interrupt_operation(&self, generation: u64, operation_id: u64) {
    if let Ok(control) = self.control.lock()
      && let Some(active) = control.active.as_ref()
      && active.generation == generation
      && active.id == operation_id
    {
      active.interrupt.interrupt();
    }
  }

  fn cancellation_requested(&self, token: &CancelToken, external: Option<&CancelToken>) -> bool {
    token.is_cancelled() || external.is_some_and(CancelToken::is_cancelled)
  }

  fn reset_opening(&self) {
    if let Ok(mut control) = self.control.lock()
      && control.lifecycle == NativeSessionLifecycle::Opening
    {
      control.lifecycle = NativeSessionLifecycle::New;
    }
  }

  fn is_retiring(&self) -> bool {
    self
      .control
      .lock()
      .map(|control| {
        matches!(
          control.lifecycle,
          NativeSessionLifecycle::Retiring | NativeSessionLifecycle::Closed
        )
      })
      .unwrap_or(true)
  }

  fn mark_retiring(&self) {
    if let Ok(mut control) = self.control.lock()
      && control.lifecycle != NativeSessionLifecycle::Closed
    {
      control.lifecycle = NativeSessionLifecycle::Retiring;
    }
    self.retirement.send_replace(true);
    self.notify_retired();
  }

  fn mark_failed(&self) {
    if let Ok(mut control) = self.control.lock() {
      control.lifecycle = NativeSessionLifecycle::Failed;
      control.active = None;
    }
    self.retirement.send_replace(true);
    self.notify_retired();
  }

  async fn close_after_drain(self: &Arc<Self>) {
    let mut activity = self.activity.subscribe();
    loop {
      if *activity.borrow_and_update() == 0 {
        break;
      }
      if activity.changed().await.is_err() {
        break;
      }
    }

    let permit = self.execution.clone().lock_owned().await;
    let session = Arc::clone(self);
    let _ = crate::dialect::run_blocking(move || {
      let _permit = permit;
      let native = match session.native.lock() {
        Ok(mut native) => native.take(),
        Err(poisoned) => poisoned.into_inner().take(),
      };
      drop(native);
      Ok(())
    })
    .await;

    if let Ok(mut control) = self.control.lock() {
      control.active = None;
      control.lifecycle = NativeSessionLifecycle::Closed;
    }
  }
}

impl Drop for NativeSession {
  fn drop(&mut self) { self.notify_retired(); }
}

struct ActivityLease {
  session: Arc<NativeSession>,
}

impl Drop for ActivityLease {
  fn drop(&mut self) {
    if let Ok(mut control) = self.session.control.lock() {
      control.admitted = control.admitted.saturating_sub(1);
      control.last_completed_at = Instant::now();
      self.session.activity.send_replace(control.admitted);
    }
  }
}

struct CallerDropGuard {
  token: CancelToken,
  session: Arc<NativeSession>,
  generation: u64,
  operation_id: u64,
  armed: bool,
}

impl Drop for CallerDropGuard {
  fn drop(&mut self) {
    if self.armed {
      self.token.cancel();
      self.session.interrupt_operation(self.generation, self.operation_id);
    }
  }
}

async fn wait_for_cancel(token: Option<CancelToken>) {
  if let Some(token) = token {
    token.cancelled().await;
  } else {
    pending::<()>().await;
  }
}

struct OperationMonitor {
  done: std::sync::mpsc::Sender<()>,
  worker: Option<std::thread::JoinHandle<()>>,
}

impl OperationMonitor {
  fn start(
    session: Arc<NativeSession>, generation: u64, operation_id: u64,
    token: CancelToken, external: Option<CancelToken>,
  ) -> anyhow::Result<Self> {
    let (done, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new().name("duckling-cancel".into()).spawn(move || {
      loop {
        if token.is_cancelled() || external.as_ref().is_some_and(CancelToken::is_cancelled) || session.is_retiring() {
          token.cancel();
          session.interrupt_operation(generation, operation_id);
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
          Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
          Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {},
        }
      }
    })?;
    Ok(Self { done, worker: Some(worker) })
  }
}

impl Drop for OperationMonitor {
  fn drop(&mut self) {
    let _ = self.done.send(());
    if let Some(worker) = self.worker.take() {
      let _ = worker.join();
    }
  }
}

fn cancelled_error() -> anyhow::Error {
  anyhow::Error::new(ConnectorError::Cancelled)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::sync::oneshot;

  #[test]
  fn runtime_shutdown_drains_an_active_native_export() {
    assert_shutdown_drains(false);
  }

  #[test]
  fn runtime_shutdown_drains_export_starting_after_caller_drop() {
    assert_shutdown_drains(true);
  }

  fn assert_shutdown_drains(pause_before_native: bool) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let session = Arc::new(NativeSession::new_local(None, None));
    let output = std::env::temp_dir().join(format!("duckling-shutdown-{}.csv", nanoid::nanoid!()));
    let worker_output = output.clone();
    let worker_session = Arc::clone(&session);
    let (entered, started) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = runtime.block_on(async {
      let task = tokio::spawn(async move {
        worker_session.run(None, move |connection, _| {
          let _ = entered.send(());
          if pause_before_native { wait.recv().unwrap(); }
          connection.export(
            "SELECT sum(i)::VARCHAR FROM range(1000000000000) AS t(i)",
            &worker_output.to_string_lossy(), "csv", &Default::default(),
          )
        }).await
      });
      started.await.unwrap();
      tokio::time::sleep(Duration::from_millis(30)).await;
      assert!(session.snapshot().active_operation.is_some());
      task
    });
    runtime.shutdown_timeout(Duration::from_millis(100));
    drop(task);
    let _ = release.send(());
    let deadline = Instant::now() + Duration::from_millis(500);
    while session.snapshot().admitted != 0 && Instant::now() < deadline {
      std::thread::sleep(Duration::from_millis(5));
    }
    let drained = session.snapshot().admitted == 0;
    if !drained {
      if let Some(active) = session.control.lock().unwrap().active.as_ref() {
        active.interrupt.interrupt();
      }
      let deadline = Instant::now() + Duration::from_secs(2);
      while session.snapshot().admitted != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
      }
    }
    if drained {
      let recovery = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
      let value = recovery.block_on(session.run(None, |connection, _| {
        Ok(connection.inner.query_row("SELECT 42::INTEGER", [], |row| row.get::<_, i32>(0))?)
      })).unwrap();
      assert_eq!(value, 42);
    }
    let _ = std::fs::remove_file(output);
    assert!(drained, "runtime shutdown left native export running after its caller and monitor were dropped");
  }

  #[tokio::test]
  async fn stale_caller_guard_does_not_interrupt_a_successor() {
    let session = Arc::new(NativeSession::new_local(None, None));
    session.run(None, |_, _| Ok(())).await.unwrap();
    let stale = CallerDropGuard {
      token: CancelToken::new(), session: Arc::clone(&session),
      generation: session.generation, operation_id: 1, armed: true,
    };
    let value = session.run(None, move |connection, _| {
      drop(stale);
      Ok(connection.inner.query_row("SELECT sum(i)::BIGINT FROM range(10000) t(i)", [], |row| row.get::<_, i64>(0))?)
    }).await.unwrap();
    assert_eq!(value, 49_995_000);
  }

  #[tokio::test]
  async fn lazy_owner_opens_once_and_retains_native_state() {
    let session = Arc::new(NativeSession::new_local(None, None));
    for _ in 0..20 {
      session
        .run(None, |connection, _| {
          connection.inner.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS retained (value INTEGER); INSERT INTO retained VALUES (7)",
          )?;
          Ok(())
        })
        .await
        .unwrap();
      let values = session
        .run(None, |connection, _| {
          connection
            .inner
            .query_row(
              "SELECT value FROM retained ORDER BY rowid DESC LIMIT 1",
              [],
              |row| row.get::<_, i64>(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();
      assert_eq!(values, 7);
    }
    assert_eq!(session.snapshot().opens, 1);
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn fifo_gate_serializes_one_owner_and_leaves_other_owners_independent() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    let session = Arc::new(NativeSession::new_local(None, None));
    let other_session = Arc::new(NativeSession::new_local(None, None));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let order = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let first_session = Arc::clone(&session);
    let first_active = Arc::clone(&active);
    let first_maximum = Arc::clone(&maximum);
    let first_order = Arc::clone(&order);
    let first = tokio::spawn(async move {
      first_session
        .run(None, move |_, _| {
          let count = first_active.fetch_add(1, Ordering::SeqCst) + 1;
          first_maximum.fetch_max(count, Ordering::SeqCst);
          first_order.lock().unwrap().push('A');
          let _ = entered_tx.send(());
          release_rx.recv().unwrap();
          first_active.fetch_sub(1, Ordering::SeqCst);
          Ok(())
        })
        .await
    });
    entered_rx.await.unwrap();

    let second_session = Arc::clone(&session);
    let second_active = Arc::clone(&active);
    let second_maximum = Arc::clone(&maximum);
    let second_order = Arc::clone(&order);
    let second = tokio::spawn(async move {
      second_session
        .run(None, move |_, _| {
          let count = second_active.fetch_add(1, Ordering::SeqCst) + 1;
          second_maximum.fetch_max(count, Ordering::SeqCst);
          second_order.lock().unwrap().push('B');
          second_active.fetch_sub(1, Ordering::SeqCst);
          Ok(())
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
      while session.snapshot().admitted < 2 {
        tokio::task::yield_now().await;
      }
    })
    .await
    .unwrap();

    let third_session = Arc::clone(&session);
    let third_active = Arc::clone(&active);
    let third_maximum = Arc::clone(&maximum);
    let third_order = Arc::clone(&order);
    let third = tokio::spawn(async move {
      third_session
        .run(None, move |_, _| {
          let count = third_active.fetch_add(1, Ordering::SeqCst) + 1;
          third_maximum.fetch_max(count, Ordering::SeqCst);
          third_order.lock().unwrap().push('C');
          third_active.fetch_sub(1, Ordering::SeqCst);
          Ok(())
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
      while session.snapshot().admitted < 3 {
        tokio::task::yield_now().await;
      }
    })
    .await
    .unwrap();

    let independent = tokio::time::timeout(
      Duration::from_secs(1),
      other_session.run(None, |_, _| Ok(9)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(independent, 9);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);

    release_tx.send(()).unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    third.await.unwrap().unwrap();
    assert_eq!(*order.lock().unwrap(), vec!['A', 'B', 'C']);
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn opening_counts_as_activity_and_idle_time_starts_at_completion() {
    let session = Arc::new(NativeSession::new_local(None, None));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    *session.open_hook.lock().unwrap() = Some(Arc::new(move || {
      entered_tx.send(()).unwrap();
      release_rx.lock().unwrap().recv().unwrap();
    }));

    let opening_session = Arc::clone(&session);
    let opening = tokio::spawn(async move { opening_session.run(None, |_, _| Ok(1)).await });
    tokio::task::spawn_blocking(move || {
      entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("native open should reach the test barrier");
    })
    .await
    .unwrap();

    assert_eq!(
      session.snapshot().lifecycle,
      NativeSessionLifecycle::Opening
    );
    assert_eq!(session.snapshot().admitted, 1);
    let future = Instant::now() + Duration::from_secs(60);
    assert!(!session.retire_if_idle(Duration::from_secs(1), future).await);
    assert!(!session.retire_if_idle(Duration::ZERO, future).await);
    assert_eq!(
      session.snapshot().lifecycle,
      NativeSessionLifecycle::Opening
    );

    release_tx.send(()).unwrap();
    assert_eq!(opening.await.unwrap().unwrap(), 1);
    let completed = session.snapshot().last_completed_at;
    assert!(
      !session
        .retire_if_idle(
          Duration::from_secs(1),
          completed + Duration::from_secs(1) - Duration::from_nanos(1),
        )
        .await
    );
    assert!(
      session
        .retire_if_idle(Duration::from_secs(1), completed + Duration::from_secs(1))
        .await
    );
    assert_eq!(session.snapshot().lifecycle, NativeSessionLifecycle::Closed);
  }

  #[tokio::test]
  async fn retirement_rejects_new_operations_and_closes_after_drain() {
    let session = Arc::new(NativeSession::new_local(None, None));
    session
      .run(None, |connection, _| {
        connection
          .inner
          .execute_batch("CREATE TEMP TABLE retained (value INTEGER)")?;
        Ok(())
      })
      .await
      .unwrap();
    session.retire().await;
    assert_eq!(session.snapshot().lifecycle, NativeSessionLifecycle::Closed);
    assert!(session.run(None, |_, _| Ok(())).await.is_err());
  }
}
