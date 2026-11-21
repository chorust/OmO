#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationDisposition {
    Current,
    Stale,
}

pub fn classify_generation(
    message_generation: u64,
    current_generation: u64,
) -> GenerationDisposition {
    if message_generation == current_generation {
        GenerationDisposition::Current
    } else {
        GenerationDisposition::Stale
    }
}
