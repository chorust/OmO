use omo_protocol::result_ref::ResultLimits;
use thiserror::Error;

pub use omo_protocol::result_ref::ResultLimits as QueryLimits;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LimitError {
    #[error("query row limit must be greater than zero")]
    ZeroRows,
    #[error("query byte limit must be greater than zero")]
    ZeroBytes,
}

pub fn validate_limits(limits: ResultLimits) -> Result<(), LimitError> {
    if limits.rows == 0 {
        return Err(LimitError::ZeroRows);
    }
    if limits.bytes == 0 {
        return Err(LimitError::ZeroBytes);
    }
    Ok(())
}

pub const fn interactive_limits() -> ResultLimits {
    ResultLimits {
        rows: 100_000,
        bytes: 64 * 1024 * 1024,
    }
}

pub const fn job_limits() -> ResultLimits {
    ResultLimits {
        rows: 10_000_000,
        bytes: 2 * 1024 * 1024 * 1024,
    }
}
