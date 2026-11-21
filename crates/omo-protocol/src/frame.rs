use std::io::{self, Read, Write};

use thiserror::Error;

pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame length {length} exceeds maximum {max}")]
    TooLarge { length: usize, max: usize },
    #[error("frame length prefix ended after {actual} of 4 bytes")]
    TruncatedLength { actual: usize },
    #[error("frame body ended after {actual} of {expected} bytes")]
    TruncatedBody { expected: usize, actual: usize },
    #[error("frame body is not valid UTF-8 JSON text")]
    InvalidUtf8,
    #[error("frame I/O failed: {0}")]
    Io(#[from] io::Error),
}

pub fn encode_frame(body: &[u8]) -> Result<Vec<u8>, FrameError> {
    if body.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge {
            length: body.len(),
            max: MAX_FRAME_LEN,
        });
    }
    std::str::from_utf8(body).map_err(|_| FrameError::InvalidUtf8)?;
    let length = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge {
        length: body.len(),
        max: MAX_FRAME_LEN,
    })?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

pub fn write_frame(mut writer: impl Write, body: &[u8]) -> Result<(), FrameError> {
    let frame = encode_frame(body)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

pub fn decode_frame(mut reader: impl Read) -> Result<Vec<u8>, FrameError> {
    let mut length_bytes = [0u8; 4];
    let mut length_read = 0;
    while length_read < length_bytes.len() {
        match reader.read(&mut length_bytes[length_read..]) {
            Ok(0) => {
                return Err(FrameError::TruncatedLength {
                    actual: length_read,
                });
            }
            Ok(n) => length_read += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FrameError::Io(error)),
        }
    }

    let expected = u32::from_le_bytes(length_bytes) as usize;
    if expected > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge {
            length: expected,
            max: MAX_FRAME_LEN,
        });
    }

    let mut body = vec![0u8; expected];
    let mut actual = 0;
    while actual < expected {
        match reader.read(&mut body[actual..]) {
            Ok(0) => return Err(FrameError::TruncatedBody { expected, actual }),
            Ok(n) => actual += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FrameError::Io(error)),
        }
    }
    std::str::from_utf8(&body).map_err(|_| FrameError::InvalidUtf8)?;
    Ok(body)
}
