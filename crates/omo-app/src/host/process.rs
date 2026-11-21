use std::{
    io::{BufReader, Read},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
};

use omo_protocol::{
    frame::{FrameError, decode_frame, write_frame},
    messages::{Envelope, MessageBody},
};
use serde_json::Value;

use super::supervisor::{WorkerEvent, WorkerFactory, WorkerPoll, WorkerProcess};

pub struct SubprocessWorkerFactory {
    executable: PathBuf,
}

impl SubprocessWorkerFactory {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl WorkerFactory for SubprocessWorkerFactory {
    fn spawn(&mut self, _generation: u64) -> Result<Box<dyn WorkerProcess>, String> {
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn {}: {error}", self.executable.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "worker stdin was not piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "worker stdout was not piped".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "worker stderr was not piped".to_owned())?;
        let (event_tx, events) = mpsc::channel();
        let (stderr_tx, stderr_chunks) = mpsc::channel();
        let stdout_thread = thread::Builder::new()
            .name("omo-worker-stdout".into())
            .spawn(move || read_frames(stdout, event_tx))
            .map_err(|error| format!("start worker stdout reader: {error}"))?;
        let stderr_thread = thread::Builder::new()
            .name("omo-worker-stderr".into())
            .spawn(move || read_stderr(stderr, stderr_tx))
            .map_err(|error| format!("start worker stderr reader: {error}"))?;

        Ok(Box::new(SubprocessWorker {
            child,
            stdin,
            events,
            stderr_chunks,
            _stdout_thread: stdout_thread,
            _stderr_thread: stderr_thread,
            exited: false,
        }))
    }
}

struct SubprocessWorker {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::Receiver<Result<WorkerEvent, String>>,
    stderr_chunks: mpsc::Receiver<Vec<u8>>,
    _stdout_thread: thread::JoinHandle<()>,
    _stderr_thread: thread::JoinHandle<()>,
    exited: bool,
}

impl WorkerProcess for SubprocessWorker {
    fn send(&mut self, request: Envelope<Value>) -> Result<(), String> {
        let bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        write_frame(&mut self.stdin, &bytes).map_err(|error| error.to_string())
    }

    fn poll(&mut self) -> Result<WorkerPoll, String> {
        let mut poll = WorkerPoll::default();
        loop {
            match self.events.try_recv() {
                Ok(Ok(event)) => poll.events.push(event),
                Ok(Err(error)) => return Err(error),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        while let Ok(chunk) = self.stderr_chunks.try_recv() {
            poll.stderr.extend(chunk);
        }
        if !self.exited {
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                self.exited = true;
                poll.exited = true;
            }
        }
        Ok(poll)
    }

    fn kill(&mut self) -> Result<(), String> {
        if !self.exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.exited = true;
        }
        Ok(())
    }
}

impl Drop for SubprocessWorker {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

fn read_frames<R: Read>(stdout: R, sender: mpsc::Sender<Result<WorkerEvent, String>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let bytes = match decode_frame(&mut reader) {
            Ok(bytes) => bytes,
            Err(FrameError::TruncatedLength { actual: 0 }) => break,
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                break;
            }
        };
        let event: Result<Envelope<MessageBody>, _> = serde_json::from_slice(&bytes);
        match event {
            Ok(event) => {
                if sender.send(Ok(WorkerEvent(event))).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(format!("invalid worker event JSON: {error}")));
                break;
            }
        }
    }
}

fn read_stderr<R: Read>(mut stderr: R, sender: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0u8; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}
