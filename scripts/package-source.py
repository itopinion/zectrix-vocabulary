"""Archive only this repository's Git-tracked source files; never a working tree dump."""
from pathlib import Path
import hashlib, re, stat, subprocess, sys, zipfile

root = Path(__file__).resolve().parents[1]
def git(*args):
    return subprocess.check_output(['git', '-C', str(root), *args])

try:
    top = Path(git('rev-parse', '--show-toplevel').decode().strip()).resolve()
except subprocess.CalledProcessError:
    raise SystemExit('Initialize this project as its own Git repository and add the intended source files first.')
if top != root:
    raise SystemExit('Source archive requires this project to be its own Git repository, not a parent checkout.')
names = [name.decode('utf-8') for name in git('ls-files', '-z', '--cached').split(b'\0') if name]
if not names or 'Cargo.toml' not in names:
    raise SystemExit('No tracked project source. Review and git add the intended files first.')
if subprocess.run(['git', '-C', str(root), 'diff', '--quiet', '--']).returncode:
    raise SystemExit('Tracked files changed after git add. Review and stage the current source before packaging.')

blocked = {'data', 'dist', 'target', '.git', 'env', 'models', 'recordings', 'diagnostics', '__pycache__', '.DS_Store', 'zfull_GB.ttf'}
suffixes = {'.wav', '.mp3', '.safetensors', '.pth', '.pt', '.pyc', '.bin', '.elf', '.hex', '.dfu', '.log'}
files = []
for name in sorted(set(names)):
    relative = Path(name)
    if relative.is_absolute() or '..' in relative.parts or any(part in blocked for part in relative.parts):
        raise SystemExit(f'Forbidden tracked source path: {name}')
    if relative.name.startswith('.env') or relative.suffix.lower() in suffixes:
        raise SystemExit(f'Private environment, recording, model or firmware in tracked source: {name}')
    file = root/relative
    if not file.is_file() or file.is_symlink():
        raise SystemExit(f'Tracked source must be a regular file: {name}')
    if not file.resolve().is_relative_to(root):
        raise SystemExit(f'Tracked source escapes repository: {name}')
    files.append((name, file))

toml = (root/'Cargo.toml').read_text()
version = re.search(r'^version\s*=\s*"([^"]+)"', toml, re.MULTILINE)
if not version or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-.][A-Za-z0-9.-]+)?', version[1]):
    raise SystemExit('Invalid package version')
prefix = f'{root.name}-{version[1]}-source'
dest = root/'dist'; dest.mkdir(exist_ok=True)
archive = dest/(prefix+'.zip')
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as out:
    for name, file in files:
        info = zipfile.ZipInfo(f'{prefix}/{name}')
        info.create_system = 3
        mode = 0o755 if file.stat().st_mode & stat.S_IXUSR else 0o644
        info.external_attr = (stat.S_IFREG|mode) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        out.writestr(info, file.read_bytes())
archive.with_suffix('.zip.sha256').write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n')
print(f'Source archive: {archive} ({len(files)} tracked files)')
