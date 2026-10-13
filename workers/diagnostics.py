"""Explicit, one-shot local capture for reproducing spelling failures."""
import json
import time
from pathlib import Path


def capture(root, request):
    folder = Path(root) / 'data' / 'diagnostics'
    marker = folder / 'capture-next-spelling'
    if request.get('mode') != 'spell' or not marker.exists():
        return None
    age = time.time() - marker.stat().st_mtime
    marker.unlink()  # One request only; never retain the normal recording stream.
    if not 0 <= age <= 900:
        return None
    dest = folder / 'spelling-sample.wav'
    dest.write_bytes(Path(request['path']).read_bytes())
    dest.chmod(0o600)
    return dest


def result(sample, value):
    if sample:
        dest = sample.with_suffix('.json')
        dest.write_text(json.dumps(value, ensure_ascii=False, indent=2))
        dest.chmod(0o600)
