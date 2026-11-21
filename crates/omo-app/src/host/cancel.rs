use std::time::{Duration, Instant};

use omo_protocol::state::{CancelLevel, MAX_DRAIN_TIMEOUT_MS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelAction {
    SendInterrupt { target_request_id: u64 },
    DrainUntil(Instant),
    KillWorker,
    AlreadyFinished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancelPhase {
    Running,
    InterruptSent { target_request_id: u64 },
    Draining { deadline: Instant },
    Killed,
    Finished(CancelLevel),
}

pub struct CancelController {
    phase: CancelPhase,
    drain_timeout: Duration,
}

impl CancelController {
    pub fn new(drain_timeout: Duration) -> Self {
        let maximum = Duration::from_millis(MAX_DRAIN_TIMEOUT_MS);
        Self {
            phase: CancelPhase::Running,
            drain_timeout: drain_timeout.min(maximum),
        }
    }

    pub fn request(
        &mut self,
        target_request_id: u64,
        app_exit: bool,
        now: Instant,
    ) -> CancelAction {
        if app_exit {
            self.phase = CancelPhase::Killed;
            return CancelAction::KillWorker;
        }
        if self.phase != CancelPhase::Running {
            return CancelAction::AlreadyFinished;
        }
        self.phase = CancelPhase::InterruptSent { target_request_id };
        let _ = now;
        CancelAction::SendInterrupt { target_request_id }
    }

    pub fn begin_drain(&mut self, now: Instant) -> CancelAction {
        let deadline = now + self.drain_timeout;
        self.phase = CancelPhase::Draining { deadline };
        CancelAction::DrainUntil(deadline)
    }

    pub fn poll(&mut self, now: Instant) -> CancelAction {
        match self.phase {
            CancelPhase::Draining { deadline } if now >= deadline => {
                self.phase = CancelPhase::Killed;
                CancelAction::KillWorker
            }
            CancelPhase::Draining { deadline } => CancelAction::DrainUntil(deadline),
            CancelPhase::Killed | CancelPhase::Finished(_) => CancelAction::AlreadyFinished,
            CancelPhase::Running | CancelPhase::InterruptSent { .. } => {
                CancelAction::AlreadyFinished
            }
        }
    }

    pub fn completed(&mut self, level: CancelLevel) {
        self.phase = CancelPhase::Finished(level);
    }

    pub fn phase_level(&self) -> Option<CancelLevel> {
        match self.phase {
            CancelPhase::InterruptSent { .. } => Some(CancelLevel::Interrupt),
            CancelPhase::Draining { deadline: _ } => Some(CancelLevel::Drain {
                timeout_ms: self
                    .drain_timeout
                    .as_millis()
                    .min(MAX_DRAIN_TIMEOUT_MS as u128) as u64,
            }),
            CancelPhase::Killed => Some(CancelLevel::Kill),
            CancelPhase::Finished(level) => Some(level),
            CancelPhase::Running => None,
        }
    }
}
