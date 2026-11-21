#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <community-signed|github-release-unsigned> <linux_amd64|linux_arm64|osx_arm64>" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
channel="$1"
platform="$2"
case "$channel" in
  community-signed|github-release-unsigned) ;;
  *) usage ;;
esac
case "$platform" in
  linux_amd64|linux_arm64|osx_arm64) ;;
  *) usage ;;
esac

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
lock_file="$repo_root/artifacts/duckomo.lock.json"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/omo-duckomo-fetch.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

if [[ "$channel" == "community-signed" ]]; then
  filename="duckomo.duckdb_extension.gz"
  url="https://community-extensions.duckdb.org/v2.0.0/${platform}/${filename}"
  curl --fail --location --silent --show-error "$url" --output "$work_dir/$filename"
  release_fields='"pr":"duckdb/community-extensions#3577","ref":"1bc76c3a36d26607b463909153cff9c255e19919"'
else
  filename="duckomo-v0.4.0-duckdb-v2.0.0-${platform}.duckdb_extension.gz"
  url="https://github.com/chorust/duckomo/releases/download/v0.4.0/${filename}"
  curl --fail --location --silent --show-error "$url" --output "$work_dir/$filename"
  curl --fail --location --silent --show-error \
    "https://github.com/chorust/duckomo/releases/download/v0.4.0/SHA256SUMS" \
    --output "$work_dir/SHA256SUMS"
  expected="$(awk -v asset="$filename" '$2 == asset || $2 == "*" asset { print $1 }' "$work_dir/SHA256SUMS")"
  if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "SHA256SUMS has no valid entry for $filename" >&2
    exit 1
  fi
  actual="$(sha256sum "$work_dir/$filename" | awk '{print $1}')"
  if [[ "${actual,,}" != "${expected,,}" ]]; then
    echo "release checksum mismatch: expected $expected, got $actual" >&2
    exit 1
  fi
  release_fields='"release_tag":"v0.4.0"'
fi

gzip -dc "$work_dir/$filename" > "$work_dir/duckomo.duckdb_extension"
compressed_sha="$(sha256sum "$work_dir/$filename" | awk '{print $1}')"
compressed_size="$(wc -c < "$work_dir/$filename" | tr -d '[:space:]')"
decompressed_sha="$(sha256sum "$work_dir/duckomo.duckdb_extension" | awk '{print $1}')"
decompressed_size="$(wc -c < "$work_dir/duckomo.duckdb_extension" | tr -d '[:space:]')"
downloaded_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

python3 - "$channel" "$platform" "$filename" "$url" "$compressed_sha" "$compressed_size" \
  "$decompressed_sha" "$decompressed_size" "$downloaded_at" "$release_fields" \
  "$lock_file" "$work_dir/duckomo.duckdb_extension" "$work_dir/community-signing-key.pem" <<'PY'
import base64
import hashlib
import json
import subprocess
import sys
from pathlib import Path

(channel, platform, filename, url, sha, size, decompressed_sha,
 decompressed_size, downloaded_at, provenance_json, lock_path,
 decompressed_path, key_path) = sys.argv[1:]
lock = json.loads(Path(lock_path).read_text())
current = next((item for item in lock["entries"] if
                item["channel"] == channel and item["version"] == "v0.4.0" and
                item["duckdb_version"] == "v2.0.0" and item["platform"] == platform), None)
if current is None:
    raise SystemExit(f"no pinned lock entry for {channel}/{platform}")

source = {"url": url, "downloaded_at": downloaded_at}
source.update(json.loads("{" + provenance_json + "}"))
signature = {}
if channel == "community-signed":
    fingerprint = current.get("signature_key_sha256")
    pem = current.get("signature_public_key_pem")
    if not fingerprint or not pem:
        raise SystemExit("community lock entry has no pinned public key")
    encoded = "".join(line.strip() for line in pem.splitlines()
                      if not line.startswith("-----"))
    der = base64.b64decode(encoded, validate=True)
    actual_fingerprint = hashlib.sha256(der).hexdigest()
    if actual_fingerprint != fingerprint:
        raise SystemExit("community public key does not match its pinned fingerprint")
    Path(key_path).write_text(pem)
    extension = Path(decompressed_path).read_bytes()
    if len(extension) <= 256:
        raise SystemExit("decompressed community extension is too short to contain a signature")
    body, signature_bytes = extension[:-256], extension[-256:]
    block_hashes = b"".join(
        hashlib.sha256(body[offset:offset + 1024 * 1024]).digest()
        for offset in range(0, len(body), 1024 * 1024)
    )
    digest_path = Path(decompressed_path).with_suffix(".signature-digest")
    signature_path = Path(decompressed_path).with_suffix(".signature")
    digest_path.write_bytes(hashlib.sha256(block_hashes).digest())
    signature_path.write_bytes(signature_bytes)
    result = subprocess.run(
        ["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", key_path,
         "-pkeyopt", "digest:sha256", "-in", str(digest_path),
         "-sigfile", str(signature_path)],
        capture_output=True, text=True,
    )
    if result.returncode:
        raise SystemExit(f"community extension signature verification failed: {result.stderr.strip()}")
    signature = {
        "signature_key_sha256": fingerprint,
        "signature_public_key_pem": pem,
    }
    print(f"community signature verified for {platform} with key sha256:{fingerprint}", file=sys.stderr)

entry = {
    "channel": channel,
    "extension": "duckomo",
    "version": "v0.4.0",
    "duckdb_version": "v2.0.0",
    "platform": platform,
    "file": filename,
    "sha256": sha,
    "size": int(size),
    "sha256_decompressed": decompressed_sha,
    "size_decompressed": int(decompressed_size),
    **signature,
    "source": source,
    "golden": current["golden"],
}
print(json.dumps(entry, indent=2, sort_keys=False))
PY
