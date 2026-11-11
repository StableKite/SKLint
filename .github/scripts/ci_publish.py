"""Fail-closed sixth-job publisher; read and write credentials never share a phase."""
from pathlib import Path, PurePosixPath
import argparse
import ast
import base64
import csv
from datetime import datetime, timezone
from email.parser import BytesParser
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import sysconfig
import tarfile
import tomllib
from urllib.parse import quote, unquote, urlsplit
import zipfile

INDEX = 'https://devpi.stablekite.com/StableKite/index/'
TARGETS = ('windows-x86_64', 'glibc-x86_64', 'glibc-aarch64', 'musl-x86_64', 'musl-aarch64')
PRODUCERS = tuple(item.replace('-', '_') for item in TARGETS)
PLATFORMS = dict(zip(TARGETS, ('win_amd64', 'manylinux_2_35_x86_64',
    'manylinux_2_35_aarch64', 'musllinux_1_2_x86_64', 'musllinux_1_2_aarch64')))
RUST_TARGETS = dict(zip(TARGETS, ('x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu',
    'aarch64-unknown-linux-gnu', 'x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl')))
RUSTC = 'rustc 1.98.0 (88d9e12ae 2026-08-18)'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    with Path(path).open('x', encoding='utf-8') as stream:
        json.dump(data, stream, indent=2)
        stream.write('\n')


def git(source, *args):
    return subprocess.check_output(['git', '-C', str(source), *args],
                                   stderr=subprocess.PIPE).decode().strip()


def literal_version(text):
    values = [node.value.value for node in ast.parse(text).body
              if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant)
              and any(isinstance(name, ast.Name) and name.id == '__version__'
                      for name in node.targets)]
    require(len(values) == 1 and isinstance(values[0], str), 'Expected one literal __version__.')
    return values[0]


def runtime_policy():
    import urllib.request
    require('PYTHON_GIL' not in os.environ and sys.flags.gil is None and
            not sys._is_gil_enabled(), 'Natural GIL OFF is required.')
    require(sysconfig.get_config_var('Py_GIL_DISABLED') == 1 and
            sys.version_info.releaselevel == 'final', 'Stable free-threaded CPython required.')
    require(os.environ.get('PYTHON_JIT') == '1', 'PYTHON_JIT=1 required.')
    available = sys._jit.is_available()
    require(not available or sys._jit.is_enabled(), 'Available JIT must be enabled.')
    with urllib.request.urlopen('https://www.python.org/api/v2/downloads/release/?is_published=true&pre_release=false', timeout=30) as response:
        versions = [tuple(map(int, match[1].split('.'))) for item in json.load(response)
                    if item['is_published'] and not item['pre_release'] and
                    (match := re.fullmatch(r'Python (3\.\d+\.\d+)', item['name']))]
    require(sys.version_info[:3] == max(versions), 'Publisher must retain newest stable FT runtime.')
    return {'status': 'PASS_LATEST_STABLE_FT_RUNTIME_POLICY',
            'version': sys.version, 'latest_final': max(versions), 'GIL': False,
            'gil_flag': None, 'JIT_mode': 'enabled' if available else
            'unavailable in selected build; latest stable t retained'}


def safe_file(root, relative):
    # Existing Windows producer manifests use native path separators.
    relative = relative.replace('\\', '/')
    name = PurePosixPath(relative)
    require(not name.is_absolute() and '..' not in name.parts and ':' not in relative,
            'Unsafe artifact path.')
    path = (root / relative).resolve()
    require(path.is_relative_to(root.resolve()) and path.is_file() and not (root / relative).is_symlink(),
            'Artifact missing or outside its root.')
    return path


def run_logged(out, name, argv, cwd, timeout):
    with (out / (name + '.stdout.log')).open('xb') as stdout, (out / (name + '.stderr.log')).open('xb') as stderr:
        result = subprocess.run(list(map(str, argv)), cwd=cwd, stdout=stdout, stderr=stderr,
                                timeout=timeout, shell=False)
    write(out / (name + '.command.json'), {'argv': list(map(str, argv)), 'cwd': str(cwd),
          'exit_code': result.returncode, 'stdout_sha256': sha(out / (name + '.stdout.log')),
          'stderr_sha256': sha(out / (name + '.stderr.log'))})
    require(result.returncode == 0, 'Integration command failed: ' + name)


def safe_url(url):
    parts = urlsplit(url)
    decoded = unquote(parts.path)
    require(parts.scheme == 'https' and parts.netloc == 'devpi.stablekite.com' and
            decoded.startswith('/StableKite/index/') and '..' not in decoded.split('/') and
            '\\' not in decoded and not parts.fragment and not parts.query,
            'Refusing non-canonical or foreign devpi URL.')
    return url


def read_session():
    import requests
    require('DEVPI_TOKEN' not in os.environ, 'Write-only upload token is forbidden in read phases.')
    token = os.environ.get('DEVPI_READ_TOKEN', '')
    require(bool(token), 'A separately authorized DEVPI_READ_TOKEN is required.')
    session = requests.Session()
    session.trust_env = False
    session.verify = True
    session.auth = ('', token)
    return session


def get_json(session, url, expected_type, allow_absent=False):
    response = session.get(safe_url(url), headers={'Accept': 'application/json'},
                           allow_redirects=False, timeout=(10, 30))
    if allow_absent and response.status_code == 404:
        require(response.headers.get('X-DEVPI-API-VERSION') == '2', 'Unverified project absence.')
        return {}
    require(response.status_code == 200 and 'application/json' in response.headers.get('Content-Type', ''),
            'Expected authenticated devpi JSON response; status ' + str(response.status_code))
    data = response.json()
    require(data.get('type') == expected_type and isinstance(data.get('result'), dict), 'Unexpected devpi response schema.')
    return data['result']


def authenticate(session):
    api = get_json(session, INDEX + '+api', 'apiconfig')
    status = api.get('authstatus')
    require(isinstance(status, list) and len(status) >= 2 and status[0] == 'ok' and status[1],
            'Read authentication was not confirmed.')
    require(isinstance(api.get('index'), str) and api['index'].rstrip('/') + '/' == INDEX,
            'Authenticated index does not match canonical endpoint.')


def check_collision(versions, current):
    from packaging.version import Version
    require(all(Version(version) < Version(current) for version in versions),
            'Existing or newer version found; refusing overwrite/downgrade.')


def collision_release(source, out):
    plan = checked_plan(source, out)
    with read_session() as session:
        authenticate(session)
        project = get_json(session, INDEX + 'sklint', 'projectconfig', allow_absent=True)
    check_collision(project, plan['current_version'])
    write(out / 'COLLISION.json', {'status': 'PASS_AUTHENTICATED_COLLISION_CHECK',
          'git_head': plan['git_head'], 'version': plan['current_version'],
          'existing_versions': sorted(project), 'endpoint': INDEX,
          'credential': 'separate read token; never DEVPI_TOKEN'})


def upload_release(source, out):
    plan = checked_plan(source, out)
    from requests.adapters import HTTPAdapter
    from twine.package import PackageFile
    from twine.repository import Repository
    manifest = read(out / 'PUBLISH_SET.json')
    require(manifest['status'] == 'PASS_EXACT_FIVE_PLATFORM_SET_AND_INTEGRATION' and
            manifest['plan_sha256'] == sha(out / 'PLAN.json') and
            manifest['collision_sha256'] == sha(out / 'COLLISION.json') and
            manifest['git_head'] == plan['git_head'] and manifest['version'] == plan['current_version'],
            'Publish set is not bound to the reviewed plan/preflight.')
    require('DEVPI_READ_TOKEN' not in os.environ, 'Read credential is forbidden in upload phase.')
    token = os.environ.get('DEVPI_TOKEN', '')
    require(bool(token), 'Upload credential missing.')
    require(len(manifest['files']) == 6 and
            {item['target'] for item in manifest['files']} == set(TARGETS) | {'canonical-source'},
            'Upload requires the exact six-artifact set.')
    paths = []
    for item in manifest['files']:
        path = safe_file(out / 'distributions', Path(item['path']).name)
        require(path.stat().st_size == item['bytes'] and sha(path) == item['sha256'],
                'Publication bytes changed after qualification.')
        paths.append(path)
    require(len({path.name for path in paths}) == 6, 'Duplicate upload filenames.')
    require(git(source, 'ls-remote', 'origin', 'refs/heads/' + plan['default_branch']).split()[0] ==
            plan['git_head'], 'Default branch advanced before upload.')

    class BoundedUploadAdapter(HTTPAdapter):
        def send(self, request, **kwargs):
            kwargs['timeout'] = (10, 90)
            return super().send(request, **kwargs)

    repository = Repository(INDEX, '', token, disable_progress_bar=True)
    repository.session.trust_env = False
    repository.session.verify = True
    # Twine normally retries 5xx. Each file here gets one non-redirecting POST.
    # Bound the socket wait without changing Requests' certificate verification.
    repository.session.mount('https://', BoundedUploadAdapter(max_retries=0))
    uploaded = []
    try:
        for item, path in zip(manifest['files'], paths):
            response = repository.upload(PackageFile.from_filename(str(path), None), max_redirects=1)
            require(response.status_code == 200 and not response.history and response.url == INDEX,
                    'Upload failed or attempted a redirect; status ' + str(response.status_code))
            uploaded.append({'name': path.name, 'sha256': item['sha256'], 'bytes': item['bytes'], 'http_status': 200})
            write(out / ('UPLOADED-' + path.name + '.json'), uploaded[-1])
    finally:
        repository.close()
    write(out / 'UPLOAD.json', {'status': 'UPLOADED_CONFIRMATION_STILL_REQUIRED',
          'git_head': plan['git_head'], 'files': uploaded, 'publish_set_sha256': sha(out / 'PUBLISH_SET.json')})


def confirm_release(source, out):
    plan = checked_plan(source, out)
    uploaded, manifest = read(out / 'UPLOAD.json'), read(out / 'PUBLISH_SET.json')
    require(uploaded['publish_set_sha256'] == sha(out / 'PUBLISH_SET.json') and
            uploaded['git_head'] == plan['git_head'], 'Upload/manifest mismatch.')
    expected = {Path(item['path']).name: item for item in manifest['files']}
    destination = out / 'confirmed-downloads'
    destination.mkdir()
    records = []
    with read_session() as session:
        authenticate(session)
        metadata = get_json(session, INDEX + 'sklint/' + quote(plan['current_version'], safe=''), 'versiondata')
        require(metadata.get('name') == 'sklint' and metadata.get('version') == plan['current_version'],
                'Published version metadata mismatch.')
        links = [link for link in metadata.get('+links', []) if link.get('rel') == 'releasefile']
        names = [unquote(PurePosixPath(urlsplit(safe_url(link['href'])).path).name) for link in links]
        require(len(names) == len(set(names)) == 6 and set(names) == set(expected), 'Published exact file set mismatch.')
        for link, name in zip(links, names):
            item = expected[name]
            if link.get('hash_spec'):
                require(link['hash_spec'] == 'sha256=' + item['sha256'], 'Server declared a different checksum.')
            with session.get(safe_url(link['href']), allow_redirects=False, timeout=(10, 30), stream=True) as response:
                require(response.status_code == 200, 'Published file readback failed.')
                count = 0
                with (destination / name).open('xb') as stream:
                    for chunk in response.iter_content(1024 * 1024):
                        count += len(chunk)
                        require(count <= item['bytes'], 'Readback exceeded expected artifact size.')
                        stream.write(chunk)
            require(count == item['bytes'] and sha(destination / name) == item['sha256'],
                    'Published server bytes differ from qualified originals.')
            records.append({'name': name, 'sha256': item['sha256'], 'bytes': count, 'url': link['href']})
    write(out / 'RESULT.json', {'status': 'PASS_AUTH_COLLISION_INTEGRATION_EXACT_SET_UPLOAD_AND_SERVER_BYTES',
          'git_head': plan['git_head'], 'version': plan['current_version'], 'files': records,
          'confirmed_utc': datetime.now(timezone.utc).isoformat()})



def version_pair(project_text, runtime_text):
    from packaging.version import Version
    project = tomllib.loads(project_text)['project']
    version = project['version']
    require(project['name'] == 'sklint' and isinstance(version, str), 'Canonical metadata mismatch.')
    require(version == literal_version(runtime_text), 'Canonical and runtime versions differ.')
    parsed = Version(version)
    require(str(parsed) == version and parsed.local is None, 'Non-canonical release version.')
    return version


def source_version(source):
    return version_pair((source / 'pyproject.toml').read_text(),
                        (source / 'python/sklint/__init__.py').read_text())


def previous_version(source, commit):
    require(re.fullmatch('[0-9a-f]{40}', commit) is not None and set(commit) != {'0'},
            'A real previous commit is required for a push.')
    return version_pair(git(source, 'show', commit + ':pyproject.toml'),
                        git(source, 'show', commit + ':python/sklint/__init__.py'))


def release_decision(event_name, ref, default, previous, current, manual=False):
    require(event_name in ('push', 'workflow_dispatch'), 'Unexpected publication event.')
    on_default = ref == 'refs/heads/' + default
    publish = on_default and ((event_name == 'workflow_dispatch' and manual) or
                             (event_name == 'push' and previous != current))
    reason = ('explicit-manual-default-publish' if publish and event_name == 'workflow_dispatch'
              else 'version-change' if publish else 'unchanged-version' if
              event_name == 'push' and previous == current else 'validation-only')
    return publish, reason


def successful_producers(value):
    require(isinstance(value, dict) and set(value) == set(PRODUCERS) and
            all(isinstance(job, dict) and job.get('result') == 'success' for job in value.values()),
            'All five original platform jobs must succeed in this run.')
    return value


def clean_source(source):
    require(not git(source, 'status', '--porcelain', '--untracked-files=no'),
            'Tracked source was modified during publication.')


def plan_release(source, out):
    require('DEVPI_TOKEN' not in os.environ and 'DEVPI_READ_TOKEN' not in os.environ,
            'No devpi credentials are allowed while planning.')
    event = read(os.environ['GITHUB_EVENT_PATH'])
    event_name, ref = os.environ['GITHUB_EVENT_NAME'], os.environ['GITHUB_REF']
    require(event['repository']['full_name'] == os.environ['GITHUB_REPOSITORY'] ==
            'StableKite/SKLint', 'Unexpected repository.')
    head = os.environ['GITHUB_SHA']
    require(git(source, 'rev-parse', 'HEAD') == head, 'Checkout/head mismatch.')
    clean_source(source)
    default = event['repository']['default_branch']
    current = source_version(source)
    previous = (previous_version(source, event['before'])
                if event_name == 'push' and ref == 'refs/heads/' + default else None)
    manual_value = event.get('inputs', {}).get('publish', 'false')
    require(manual_value in (True, False, 'true', 'false'), 'Invalid manual publish input.')
    manual = manual_value is True or manual_value == 'true'
    publish, reason = release_decision(event_name, ref, default, previous, current, manual)
    result = {'status': 'PUBLISH_PLANNED' if publish else 'PASS_VALIDATION_ONLY_NO_REPUBLISH',
              'git_head': head, 'repository': 'StableKite/SKLint', 'default_branch': default,
              'event': event_name, 'ref': ref, 'run_id': os.environ['GITHUB_RUN_ID'],
              'run_attempt': os.environ['GITHUB_RUN_ATTEMPT'], 'previous_version': previous,
              'current_version': current, 'should_publish': publish, 'reason': reason,
              'producers': successful_producers(json.loads(os.environ['NEEDS_JSON'])),
              'runtime': runtime_policy(), 'endpoint': INDEX}
    write(out / 'PLAN.json', result)
    with Path(os.environ['GITHUB_OUTPUT']).open('a') as stream:
        stream.write('should_publish=' + str(publish).lower() + '\n')
    print(json.dumps(result))


def checked_plan(source, out):
    plan = read(out / 'PLAN.json')
    require(plan['should_publish'] is True, 'Network/artifact publication phase forbidden for validation-only.')
    require(plan['git_head'] == os.environ['GITHUB_SHA'] == git(source, 'rev-parse', 'HEAD'),
            'Publication plan/head mismatch.')
    require(plan['repository'] == os.environ['GITHUB_REPOSITORY'] == 'StableKite/SKLint' and
            plan['run_id'] == os.environ['GITHUB_RUN_ID'] and
            plan['run_attempt'] == os.environ['GITHUB_RUN_ATTEMPT'], 'Publication run mismatch.')
    require(plan['ref'] == os.environ['GITHUB_REF'] == 'refs/heads/' + plan['default_branch'],
            'Publication must use the exact default branch.')
    require(plan['current_version'] == source_version(source) and plan['endpoint'] == INDEX,
            'Publication metadata changed after planning.')
    successful_producers(plan['producers'])
    clean_source(source)
    return plan


def wheel_record(archive):
    files = [item.filename for item in archive.infolist() if not item.is_dir()]
    require(len(files) == len(set(files)) and archive.testzip() is None, 'Duplicate or corrupt wheel members.')
    require(all(not PurePosixPath(name).is_absolute() and '..' not in PurePosixPath(name).parts
                and '\\' not in name and ':' not in name for name in files), 'Unsafe wheel member.')
    record, = [name for name in files if name.endswith('.dist-info/RECORD')]
    rows = list(csv.reader(io.StringIO(archive.read(record).decode())))
    require(all(len(row) == 3 for row in rows) and len(rows) == len({row[0] for row in rows})
            and {row[0] for row in rows} == set(files), 'RECORD does not cover the exact payload.')
    for name, encoded, size in rows:
        if name == record:
            require(encoded == size == '', 'RECORD self-entry must be unhashed.')
            continue
        data = archive.read(name)
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode()
        require(encoded == 'sha256=' + digest and size == str(len(data)), 'Wheel RECORD checksum mismatch.')
    return files


def check_wheel(path, version, target, head):
    from packaging.utils import parse_wheel_filename
    from packaging.tags import parse_tag
    name, parsed, _, tags = parse_wheel_filename(path.name)
    require(name == 'sklint' and str(parsed) == version and
            {str(tag) for tag in tags} == {'py3-none-' + PLATFORMS[target]}, 'Wheel filename mismatch.')
    with zipfile.ZipFile(path) as archive:
        files = wheel_record(archive)
        metadata, = [name for name in files if name.endswith('.dist-info/METADATA')]
        wheel, = [name for name in files if name.endswith('.dist-info/WHEEL')]
        package = BytesParser().parsebytes(archive.read(metadata))
        require(package['Name'] == 'sklint' and package['Version'] == version, 'Wheel metadata mismatch.')
        inner_tags = set().union(*(parse_tag(tag) for tag in
            BytesParser().parsebytes(archive.read(wheel)).get_all('Tag', [])))
        require(inner_tags == tags, 'Internal wheel tags differ.')
        info_name, = [name for name in files if name.endswith('/sklint/bin/BUILD_INFO.txt')]
        info = dict(line.split('=', 1) for line in archive.read(info_name).decode().splitlines())
        expected = {'version': version, 'revision': 'v' + version, 'source_commit': head,
                    'source_dirty': 'false', 'rustc': RUSTC, 'target': RUST_TARGETS[target],
                    'wheel_platform_tag': PLATFORMS[target]}
        require(all(info.get(key) == value for key, value in expected.items()) and
                re.fullmatch('[0-9a-f]{64}', info.get('source_tree_sha256', '')) is not None,
                'Wheel native provenance mismatch.')
        marker, = [name for name in files if name.endswith('/sklint/bin/TARGET')]
        require(archive.read(marker).decode().strip() == RUST_TARGETS[target], 'Bundled TARGET mismatch.')
        binary, = [name for name in files if name.endswith('/sklint/bin/' + ('sklint.exe' if target.startswith('windows') else 'sklint'))]
        binary_sha = hashlib.sha256(archive.read(binary)).hexdigest()
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size,
            'target': target, 'native_sha256': binary_sha, 'provenance': info,
            'RECORD_entries': len(files), 'tags': sorted(str(tag) for tag in tags)}


def check_sdist(path, version, source):
    essential = ('pyproject.toml', 'Cargo.toml', 'python/sklint/__init__.py',
                 'crates/sklint-core/src/syntax_rules.rs')
    with tarfile.open(path, 'r:gz') as archive:
        members = archive.getmembers()
        names = [item.name for item in members]
        require(len(names) == len(set(names)) and all(not item.issym() and not item.islnk()
            and (item.isfile() or item.isdir()) and not PurePosixPath(item.name).is_absolute()
            and '..' not in PurePosixPath(item.name).parts for item in members), 'Unsafe sdist payload.')
        prefix = 'sklint-' + version + '/'
        require(all(name == prefix[:-1] or name.startswith(prefix) for name in names), 'Wrong sdist root.')
        metadata = BytesParser().parsebytes(archive.extractfile(prefix + 'PKG-INFO').read())
        require(metadata['Name'] == 'sklint' and metadata['Version'] == version, 'Sdist metadata mismatch.')
        for name in essential:
            require(archive.extractfile(prefix + name).read().replace(b'\r\n', b'\n') ==
                    (source / name).read_bytes().replace(b'\r\n', b'\n'), 'Sdist source mismatch: ' + name)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size, 'target': 'canonical-source'}


def collect_artifacts(source, artifacts, version, head):
    expected = {f'sklint-{version}-py3-none-{tag}.whl': target for target, tag in PLATFORMS.items()}
    expected[f'sklint-{version}.tar.gz'] = 'canonical-source'
    entries = list(artifacts.iterdir())
    require({p.name for p in entries} == set(expected) and len(entries) == 6 and
            all(p.is_file() and not p.is_symlink() for p in entries), 'Exact six-artifact set required.')
    records = []
    for name, target in expected.items():
        path = safe_file(artifacts, name)
        records.append(check_sdist(path, version, source) if target == 'canonical-source' else
                       check_wheel(path, version, target, head))
    return records


def validate_release(source, out, artifacts):
    plan = checked_plan(source, out)
    collision = read(out / 'COLLISION.json')
    require(collision['status'] == 'PASS_AUTHENTICATED_COLLISION_CHECK' and
            collision['git_head'] == plan['git_head'] and collision['version'] == plan['current_version']
            and collision['endpoint'] == INDEX, 'Missing exact-head collision preflight.')
    records = collect_artifacts(source, artifacts, plan['current_version'], plan['git_head'])
    run_logged(out, 'original-exact-set', [sys.executable, '-I', source / '.github/scripts/ci_sklint.py',
        'verify-publish-set', '--dir', artifacts], source, 60)
    run_logged(out, 'twine-check', [sys.executable, '-m', 'twine', 'check', *[r['path'] for r in records]], source, 120)
    consumer = out / 'consumer'
    run_logged(out, 'consumer-venv', [sys.executable, '-m', 'venv', consumer], source, 180)
    python = consumer / ('Scripts/python.exe' if os.name == 'nt' else 'bin/python')
    wheel = next(r for r in records if r['target'] == 'glibc-x86_64')
    run_logged(out, 'consumer-install', [python, '-m', 'pip', 'install', '--no-index', '--no-deps', wheel['path']], source, 180)
    run_logged(out, 'original-runtime-policy', [python, '-I', source / '.github/scripts/ci_python_policy.py', '--verify'], source, 180)
    run_logged(out, 'consumer-pip-check', [python, '-m', 'pip', 'check'], source, 60)
    run_logged(out, 'original-installed-smoke', [python, '-I', source / '.github/scripts/ci_sklint.py', 'smoke'], source, 180)
    inspect = "import importlib.metadata as m,json,pathlib;d=m.distribution('sklint');p=[d.locate_file(f) for f in d.files if str(f).endswith('sklint/bin/sklint')];assert len(p)==1;print(json.dumps({'binary':str(p[0].resolve())}))"
    run_logged(out, 'consumer-location', [python, '-I', '-c', inspect], source, 60)
    binary = Path(json.loads((out / 'consumer-location.stdout.log').read_text())['binary'])
    require(binary.is_relative_to(consumer) and sha(binary) == wheel['native_sha256'], 'Installed native binary differs.')
    native = subprocess.check_output([str(binary), '--version', '--verbose'], text=True)
    write(out / 'CONSUMER_NATIVE.json', {'path': str(binary), 'sha256': sha(binary), 'version_output': native})
    for key, value in {'source-commit': plan['git_head'], 'source-dirty': 'false',
                       'source-tree-sha256': wheel['provenance']['source_tree_sha256'],
                       'target': RUST_TARGETS['glibc-x86_64'], 'rustc': RUSTC}.items():
        require(key + ': ' + value in native.splitlines(), 'Installed provenance mismatch: ' + key)
    destination = out / 'distributions'
    destination.mkdir()
    for item in records:
        path = destination / Path(item['path']).name
        shutil.copyfile(item['path'], path)
        require(sha(path) == item['sha256'], 'Staged artifact differs.')
        item['path'] = str(path)
    write(out / 'PUBLISH_SET.json', {'status': 'PASS_EXACT_FIVE_PLATFORM_SET_AND_INTEGRATION',
        'plan_sha256': sha(out / 'PLAN.json'), 'collision_sha256': sha(out / 'COLLISION.json'),
        'git_head': plan['git_head'], 'version': plan['current_version'], 'files': records})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('plan', 'collision', 'validate', 'upload', 'confirm'))
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--artifacts', type=Path)
    args = parser.parse_args()
    source, out = args.source.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.phase == 'validate':
        require(args.artifacts is not None, '--artifacts is required.')
        validate_release(source, out, args.artifacts.resolve())
    else:
        {'plan': plan_release, 'collision': collision_release,
         'upload': upload_release, 'confirm': confirm_release}[args.phase](source, out)


if __name__ == '__main__':
    main()
