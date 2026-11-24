#!/usr/bin/env python3
"""Exercise a worker with the current host's explicitly supplied locked artifact."""
import argparse
import csv
import hashlib
import io
import json
import os
import platform
import selectors
import struct
import subprocess
import tempfile
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--artifact', required=True, type=Path,
                    help='compressed artifact whose bytes match the selected lock entry')
parser.add_argument('--worker', type=Path, default=Path('target/debug/omo-worker'))
args = parser.parse_args()
root = Path.cwd()
lock = json.loads((root / 'artifacts/duckomo.lock.json').read_text())
machine = platform.machine().lower()
if platform.system() == 'Linux' and machine in ('aarch64', 'arm64'):
    host = 'linux_arm64'
elif platform.system() == 'Linux' and machine in ('x86_64', 'amd64'):
    host = 'linux_amd64'
elif platform.system() == 'Darwin' and machine in ('aarch64', 'arm64'):
    host = 'osx_arm64'
else:
    raise RuntimeError(f'unsupported live-check host: {platform.system()}/{machine}')
entry = next(e for e in lock['entries'] if e['channel'] == 'github-release-unsigned' and e['platform'] == host)
artifact_bytes = args.artifact.read_bytes()
if len(artifact_bytes) != entry['size'] or hashlib.sha256(artifact_bytes).hexdigest() != entry['sha256']:
    raise ValueError('artifact bytes do not match the selected lock entry')


def frame(kind, request_id, body):
    data = json.dumps({'v': 1, 'worker_generation': 7, 'job_id': 1,
                       'request_id': request_id, 'kind': kind, 'body': body}).encode()
    return struct.pack('<I', len(data)) + data


with tempfile.TemporaryDirectory(prefix='omo-live-worker-') as tmp:
    artifact_path = Path(tmp) / entry['file']
    artifact_path.write_bytes(artifact_bytes)
    data_dir = Path(tmp) / 'data'
    data_dir.mkdir()
    ext_cache_dir = Path(tmp) / 'ext-cache'
    ext_cache_dir.mkdir(mode=0o700)
    config = {
        'data_dir': str(data_dir),
        'ext_cache_dir': str(ext_cache_dir),
        'artifact_path': str(artifact_path),
        'fixture_path': str((root / entry['golden']['fixture']).resolve()),
        'artifact': entry,
        'platform': host,
        'threads': 1,
        'no_network': True,
    }
    proc = subprocess.Popen([str(args.worker.resolve())], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)

    def send(kind, request_id, body):
        proc.stdin.write(frame(kind, request_id, body))
        proc.stdin.flush()

    def event(deadline):
        if not selector.select(max(0, deadline - time.monotonic())):
            raise TimeoutError('worker event timeout')

        def read_exact(size):
            result = b''
            while len(result) < size:
                chunk = os.read(proc.stdout.fileno(), size - len(result))
                if not chunk:
                    raise EOFError('worker stdout closed')
                result += chunk
            return result

        length = struct.unpack('<I', read_exact(4))[0]
        if length > 16 * 1024 * 1024:
            raise ValueError('oversized frame')
        return json.loads(read_exact(length))

    def wait(kind, request_id, timeout=30):
        deadline = time.monotonic() + timeout
        seen = []
        while time.monotonic() < deadline:
            item = event(deadline)
            seen.append(item['kind'])
            if item['kind'] == kind and item['request_id'] == request_id:
                return item, seen
            if item['kind'] in ('failed', 'bootstrap_failed') and item['request_id'] == request_id:
                raise RuntimeError(str(item))
        raise TimeoutError(str(seen))

    try:
        send('hello', 1, {'bootstrap_config': config})
        ready, _ = wait('ready', 1)
        send('query', 2, {'sql': 'SELECT i FROM range(1000) t(i)', 'purpose': 'preview',
                          'limits': {'rows': 1000, 'bytes': 1048576}})
        result, _ = wait('result', 2)
        ref = result['body']['result_ref']
        assert ref['generation'] == 7 and ref['rows'] == 1000 and Path(ref['path']).is_file()

        export_path = data_dir / 'result.csv'
        send('export', 6, {'result_id': ref['id'], 'target': str(export_path), 'format': 'csv',
                           'job_snapshot': {'job_id': 1}})
        export_event, _ = wait('export_committed', 6)
        export_sidecar = Path(str(export_path) + '.omo.json')
        export_bytes = export_path.read_bytes()
        export_hash = hashlib.sha256(export_bytes).hexdigest()
        assert export_event['body']['data_sha256'] == export_hash
        assert export_sidecar.is_file()
        sidecar = json.loads(export_sidecar.read_text())
        assert sidecar['schema'] == 'omo-export-sidecar/1'
        assert sidecar['data']['sha256'] == export_hash
        assert sidecar['data']['file'] == export_path.name and sidecar['data']['rows'] == 1000
        rows = list(csv.reader(io.StringIO(export_bytes.decode())))
        assert len(rows) == 1001 and rows[0] == ['i'] and rows[1] == ['0'] and rows[-1] == ['999']

        send('query', 3, {'sql': 'SELECT sum(a.i*b.i) FROM range(1000000) a(i), range(1000000) b(i)',
                          'purpose': 'job', 'limits': {'rows': 10, 'bytes': 1048576}})
        time.sleep(1.5)
        send('interrupt', 4, {'target_request_id': 3})
        deadline = time.monotonic() + 20
        seen = []
        while True:
            item = event(deadline)
            seen.append(item['kind'])
            if item['kind'] == 'failed' and item['request_id'] == 3:
                assert item['body']['kind'] == {
                    'kind': 'cancelled',
                    'detail': {'level': 'interrupt'},
                }, item
                break
            if time.monotonic() >= deadline:
                raise TimeoutError(str(seen))
        assert 'heartbeat' in seen, seen
        send('shutdown', 5, {})
        assert proc.wait(timeout=10) == 0
        print(json.dumps({
            'status': 'passed',
            'platform': host,
            'channel': entry['channel'],
            'bootstrap': ready['body']['bootstrap_report']['golden'],
            'result_rows': ref['rows'],
            'generation': ref['generation'],
            'export': {'rows': len(rows) - 1, 'path': str(export_path),
                       'data_sha256': export_hash, 'sidecar': str(export_sidecar)},
            'cancellation': 'interrupt',
            'events_during_query': seen,
        }, indent=2))
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        selector.close()
