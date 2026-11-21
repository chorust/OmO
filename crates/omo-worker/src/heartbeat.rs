use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use omo_protocol::{frame::write_frame, messages::Envelope};
use serde_json::json;

pub struct HeartbeatHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl HeartbeatHandle {
    pub fn stop(self) {
        self.stop.store(true, Ordering::Release);
        self.thread.thread().unpark();
        let _ = self.thread.join();
    }
}

pub fn start<W>(writer: Arc<Mutex<W>>, generation: Arc<AtomicU64>) -> HeartbeatHandle
where
    W: Write + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let start = Instant::now();
        while !thread_stop.load(Ordering::Acquire) {
            thread::park_timeout(Duration::from_secs(1));
            if thread_stop.load(Ordering::Acquire) {
                break;
            }
            let event = Envelope::new(
                generation.load(Ordering::Acquire),
                0,
                0,
                "heartbeat",
                json!({"t_ms": start.elapsed().as_millis() as u64}),
            );
            let Ok(bytes) = serde_json::to_vec(&event) else {
                continue;
            };
            if let Ok(mut output) = writer.lock() {
                if write_frame(&mut *output, &bytes).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });
    HeartbeatHandle { stop, thread }
}

pub fn stdout_writer() -> Arc<Mutex<io::Stdout>> {
    Arc::new(Mutex::new(io::stdout()))
}
