use std::{
    io::{Cursor, Write},
    sync::{Arc, Mutex},
};

use omo_protocol::{
    frame::{decode_frame, encode_frame},
    messages::Envelope,
};
use omo_worker::server::{BackendError, WorkerBackend, WorkerExit, serve};
use serde_json::{Value, json};

struct NoisyBackend;

impl WorkerBackend for NoisyBackend {
    fn bootstrap(&mut self, _config: &Value, diagnostics: &mut dyn Write) -> Result<Value, String> {
        for index in 0..2_000 {
            writeln!(diagnostics, "fake diagnostic line {index}").unwrap();
        }
        Err("fake bootstrap failure".into())
    }

    fn probe_task(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn query(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Ok(json!({}))
    }

    fn interrupt(&mut self, _request_id: u64) -> Result<(), String> {
        Ok(())
    }

    fn ensure_spatial(&mut self) -> Result<Value, String> {
        Ok(json!({"spatial": true}))
    }
}

#[test]
fn log_pressure_stays_on_stderr_and_stdout_contains_only_protocol_frames() {
    let request = Envelope::new(1, 0, 1, "hello", json!({"bootstrap_config": {}}));
    let input = Cursor::new(encode_frame(&serde_json::to_vec(&request).unwrap()).unwrap());
    let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut stderr = Vec::new();

    let exit = serve(input, Arc::clone(&stdout), &mut stderr, NoisyBackend).unwrap();
    assert_eq!(exit, WorkerExit::BootstrapFailed);
    assert!(stderr.len() > 50_000);

    let output = stdout.lock().unwrap();
    let mut cursor = Cursor::new(output.as_slice());
    let bytes = decode_frame(&mut cursor).unwrap();
    let event: Envelope<Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(event.kind, "bootstrap_failed");
    assert_eq!(cursor.position() as usize, output.len());
}
