use omo_engine::lock::{ArtifactLock, ArtifactLockEntry, ArtifactMismatch};
use serde_json::json;

const COMMUNITY_KEY_SHA256: &str =
    "b9aac70a9b1de896079041d76691284aceec44c4eef75a50c90ab2e99f9d54d1";
const COMMUNITY_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAtXl28loGwAH3ZGQXXgJQ\n3omhIEiUb3z9Petjl+jmdtEQnMNUFEZiXkfJB02UFWBL1OoKKnjiGhcr5oGiIZKR\nCoaL6SfmWe//7o8STM44stE0exzZcv8W4tWwjrzSWQnwh2JgSnHN64xoDQjdvG3X\n9uQ1xXMXghWOKqEpgArpJQkHoPW3CD5sCS2NLFrBG6KgX0W+GTV5HaKhTMr2754F\nl260drcBJZhLFCeesze2DXtQC+R9D25Zwn2ehHHd2Fd1M10ZL/iKN8NeerB4Jnph\nw6E3orA0DusDLDLtpJUHhmpLoU/1eYQFQOpGw2ce5I88Tkx7SKnCRy1UiE7BA82W\nYQIDAQAB\n-----END PUBLIC KEY-----";

fn entry() -> ArtifactLockEntry {
    serde_json::from_value(json!({
        "channel":"community-signed",
        "extension":"duckomo",
        "version":"v0.4.0",
        "duckdb_version":"v2.0.0",
        "platform":"linux_arm64",
        "file":"duckomo.duckdb_extension.gz",
        "sha256":"a".repeat(64),
        "size":12,
        "sha256_decompressed":"b".repeat(64),
        "size_decompressed":99,
        "signature_key_sha256":COMMUNITY_KEY_SHA256,
        "signature_public_key_pem":COMMUNITY_KEY_PEM,
        "source":{"url":"https://example.invalid/ext.gz","ref":"abc","downloaded_at":"2026-11-16T00:00:00Z"},
        "golden":{"fixture":"fixtures/om/test.om","query_id":"golden-v1","result_sha256":"c".repeat(64)}
    })).unwrap()
}

#[test]
fn lookup_uses_all_four_key_fields() {
    let lock = ArtifactLock {
        schema: "omo-artifact-lock/1".into(),
        entries: vec![entry()],
    };
    assert!(
        lock.lookup("community-signed", "v0.4.0", "v2.0.0", "linux_arm64")
            .is_ok()
    );
    assert!(matches!(
        lock.lookup("community-signed", "v0.4.1", "v2.0.0", "linux_arm64"),
        Err(ArtifactMismatch::MissingEntry { .. })
    ));
}

#[test]
fn missing_entry_is_rejected() {
    let lock = ArtifactLock {
        schema: "omo-artifact-lock/1".into(),
        entries: vec![],
    };
    assert!(matches!(
        lock.lookup("community-signed", "v0.4.0", "v2.0.0", "linux_arm64"),
        Err(ArtifactMismatch::MissingEntry { .. })
    ));
}

#[test]
fn community_signer_fingerprint_must_match_the_pinned_public_key() {
    let mut item = entry();
    item.signature_key_sha256 = Some("0".repeat(64));
    assert!(matches!(
        item.validate(),
        Err(ArtifactMismatch::Mismatch {
            field: "signature_key_sha256",
            ..
        })
    ));
}

#[test]
fn community_signed_entry_requires_a_pinned_public_key() {
    let mut item = entry();
    item.signature_public_key_pem = None;
    assert!(matches!(
        item.validate(),
        Err(ArtifactMismatch::InvalidMetadata {
            field: "signature_public_key_pem"
        })
    ));
}

#[test]
fn unsigned_entry_cannot_carry_community_signing_material() {
    let mut item = entry();
    item.channel = "github-release-unsigned".into();
    assert!(matches!(
        item.validate(),
        Err(ArtifactMismatch::InvalidMetadata {
            field: "signature_key_sha256"
        })
    ));
}

#[test]
fn placeholder_golden_hash_is_not_ready() {
    let mut item = entry();
    item.golden.result_sha256 = "<filled after first verified run>".into();
    assert!(matches!(
        item.validate(),
        Err(ArtifactMismatch::PlaceholderGoldenHash)
    ));
}

#[test]
fn decompressed_size_and_hash_are_both_checked_before_load_callback() {
    use sha2::{Digest, Sha256};
    let bytes = b"verified artifact bytes";
    let mut item = entry();
    item.size_decompressed = bytes.len() as u64;
    item.sha256_decompressed = hex::encode(Sha256::digest(bytes));
    let mut load_called = false;
    let result = item.verify_and_then(b"tampered artifact bytes", |_| {
        load_called = true;
    });
    assert!(matches!(result, Err(ArtifactMismatch::Mismatch { .. })));
    assert!(!load_called, "invalid artifact must never reach the loader");
    item.verify_decompressed(bytes).unwrap();
}

#[test]
fn repository_lockfile_passes_strict_lookup_for_all_six_targets() {
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/duckomo.lock.json");
    let bytes = std::fs::read(&lock_path).expect("read artifacts/duckomo.lock.json");
    let lock = ArtifactLock::from_json(&bytes).expect("parse artifacts/duckomo.lock.json");
    assert_eq!(lock.entries.len(), 6);
    for channel in ["community-signed", "github-release-unsigned"] {
        for platform in ["linux_amd64", "linux_arm64", "osx_arm64"] {
            let entry = lock
                .lookup(channel, "v0.4.0", "v2.0.0", platform)
                .unwrap_or_else(|err| panic!("lookup failed for {channel}/{platform}: {err}"));
            assert_eq!(
                entry.golden.result_sha256,
                "bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f"
            );
        }
    }
}
