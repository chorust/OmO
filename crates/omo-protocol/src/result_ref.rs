use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResultRef {
    pub id: String,
    pub generation: u64,
    pub path: PathBuf,
    pub rows: u64,
    pub schema_hash: String,
    pub truncated: bool,
    pub limits: ResultLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResultLimits {
    pub rows: u64,
    pub bytes: u64,
}

impl ResultRef {
    pub fn belongs_to_generation(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

/// Hash a formatted schema debug representation (`format!("{schema:#?}")`) using lowercase hex SHA-256.
pub fn schema_hash_from_debug(debug_repr: &str) -> String {
    format!("{:x}", Sha256::digest(debug_repr.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::schema_hash_from_debug;

    #[test]
    fn schema_hash_matches_known_sha256_vector() {
        assert_eq!(
            schema_hash_from_debug("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
