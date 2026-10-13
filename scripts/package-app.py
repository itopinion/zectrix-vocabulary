"""Developer-only ZIP builder. Hub installs the resulting file without Python."""
from pathlib import Path
import hashlib, json, re, stat, sys, zipfile

root = Path(sys.argv[1]).resolve()
dest = Path(sys.argv[2]).resolve()
manifest = json.loads((root/'app.json').read_text())
if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-.][A-Za-z0-9.-]+)?', manifest['version']):
    raise SystemExit('Invalid app version')
forbidden = {'data', '.git', '__pycache__', '.DS_Store', '.env', 'recordings', 'diagnostics', 'models', 'verification', 'zfull_GB.ttf'}
files = {}
for file in sorted(root.rglob('*')):
    relative = file.relative_to(root)
    if any(part in forbidden for part in relative.parts) or file.is_symlink():
        raise SystemExit(f'Forbidden release path: {relative}')
    if not file.is_file():
        continue
    if file.suffix.lower() in {'.wav', '.mp3', '.safetensors', '.pth', '.pt', '.pyc', '.bin', '.elf', '.hex', '.dfu'}:
        raise SystemExit(f'Recording/model/cache in release: {relative}')
    if relative.as_posix() != 'app.json':
        files[relative.as_posix()] = hashlib.sha256(file.read_bytes()).hexdigest()
if manifest['entry'] not in files:
    raise SystemExit('Missing executable entry')
manifest['files'] = files
(root/'app.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2)+'\n')
archive = dest/f"zectrix-vocabulary-{manifest['version']}-macos-aarch64.zip"
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as out:
    for name in ['app.json', *files]:
        file = root/name
        info = zipfile.ZipInfo(name)
        info.create_system = 3
        mode = 0o755 if name == manifest['entry'] else 0o644
        info.external_attr = (stat.S_IFREG | mode) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        out.writestr(info, file.read_bytes())
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n')
print(f'应用包：{archive}')
