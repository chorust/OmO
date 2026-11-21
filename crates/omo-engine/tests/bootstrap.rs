use omo_engine::{
    bootstrap::{BootstrapError, Channel},
    lock::{ArtifactLockEntry, ArtifactSource, GoldenLock},
    probe::verify_extension_version,
    test_support::{new_unbootstrapped_conn, run_channel_load},
};
use sha2::{Digest, Sha256};

const COMMUNITY_KEY_SHA256: &str =
    "b9aac70a9b1de896079041d76691284aceec44c4eef75a50c90ab2e99f9d54d1";
const COMMUNITY_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAtXl28loGwAH3ZGQXXgJQ\n3omhIEiUb3z9Petjl+jmdtEQnMNUFEZiXkfJB02UFWBL1OoKKnjiGhcr5oGiIZKR\nCoaL6SfmWe//7o8STM44stE0exzZcv8W4tWwjrzSWQnwh2JgSnHN64xoDQjdvG3X\n9uQ1xXMXghWOKqEpgArpJQkHoPW3CD5sCS2NLFrBG6KgX0W+GTV5HaKhTMr2754F\nl260drcBJZhLFCeesze2DXtQC+R9D25Zwn2ehHHd2Fd1M10ZL/iKN8NeerB4Jnph\nw6E3orA0DusDLDLtpJUHhmpLoU/1eYQFQOpGw2ce5I88Tkx7SKnCRy1UiE7BA82W\nYQIDAQAB\n-----END PUBLIC KEY-----";

fn entry_for(bytes: &[u8]) -> ArtifactLockEntry {
    ArtifactLockEntry {
        channel: "community-signed".into(),
        extension: "duckomo".into(),
        version: "v0.4.0".into(),
        duckdb_version: "v2.0.0".into(),
        platform: "linux_arm64".into(),
        file: "duckomo.duckdb_extension.gz".into(),
        sha256: hex::encode(Sha256::digest(bytes)),
        size: bytes.len() as u64,
        sha256_decompressed: "a".repeat(64),
        size_decompressed: 1,
        signature_key_sha256: Some(COMMUNITY_KEY_SHA256.into()),
        signature_public_key_pem: Some(COMMUNITY_KEY_PEM.into()),
        source: ArtifactSource {
            url: "https://example.invalid/duckomo.duckdb_extension.gz".into(),
            pr: Some("duckdb/community-extensions#3577".into()),
            git_ref: Some("abc123".into()),
            release_tag: None,
            downloaded_at: "2026-11-16T00:00:00Z".into(),
        },
        golden: GoldenLock {
            fixture: "fixtures/om/test.om".into(),
            query_id: "golden-v1".into(),
            result_sha256: "b".repeat(64),
        },
    }
}

#[test]
fn compressed_size_or_hash_mismatch_prevents_any_extension_load() {
    let entry = entry_for(b"abc");
    for tampered in [b"ab".as_slice(), b"abd".as_slice()] {
        let mut load_called = false;
        let result = entry.verify_compressed_and_then(tampered, |_| {
            load_called = true;
        });
        assert!(matches!(
            result,
            Err(omo_engine::lock::ArtifactMismatch::Mismatch { .. })
        ));
        assert!(!load_called, "DuckDB must not see an unverified artifact");
    }
}

#[test]
fn wrong_extension_version_is_a_probe_failure() {
    let error = verify_extension_version("v0.4.0", "v0.3.9").unwrap_err();
    assert!(matches!(
        error,
        BootstrapError::ProbeFailed {
            probe: "duckomo_version",
            ..
        }
    ));
}

#[test]
fn signed_channel_load_failure_never_retries_unsigned() {
    let mut attempts = Vec::new();
    let result = run_channel_load(Channel::CommunitySigned, |channel| {
        attempts.push(channel);
        Err("signature verification failed".to_owned())
    });
    assert!(matches!(result, Err(BootstrapError::Duckomo { .. })));
    assert_eq!(attempts, [Channel::CommunitySigned]);
}

#[test]
fn failed_pre_task_probe_discards_the_connection() {
    let mut conn = new_unbootstrapped_conn().unwrap();
    assert!(conn.has_connection());
    let result = conn.probe_before_task();
    assert!(matches!(result, Err(BootstrapError::ProbeFailed { .. })));
    assert!(
        !conn.has_connection(),
        "a failed probe must retire the connection"
    );
}
