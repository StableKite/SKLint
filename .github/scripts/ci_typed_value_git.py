"""Independent public Git consumer; all fixtures here are owned package tests."""
from pathlib import Path
from hashlib import sha256
import base64
import json
import os
import subprocess
import sys

HEAD = '25b0ef18b3ec3061d8c4371fb9982800229ccefe'
REF = 'engineering/typed-error-values-v1'
root = Path(sys.argv[1]).absolute()
assert not root.exists()
root.mkdir()
out = root / 'evidence'
out.mkdir()
consumer = root / 'consumer'
python = consumer / 'bin/python'
source = Path.cwd()
assert sys.platform == 'linux' and sys.flags.gil is None and not sys._is_gil_enabled()
assert not any(name.startswith('SKLINT_') for name in os.environ)
env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_TARGET_DIR=str(root / 'cargo-target'))
rows = []

def run(name, args, cwd=root, required=True):
    args = list(map(str, args))
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, timeout=1200)
    (out / (name + '.stdout.log')).write_bytes(result.stdout)
    (out / (name + '.stderr.log')).write_bytes(result.stderr)
    rows.append({'name': name, 'argv': args, 'exit_code': result.returncode})
    (out / 'STAGES.json').write_text(json.dumps(rows, indent=2) + '\n')
    print(name, result.returncode, flush=True)
    if required:
        result.check_returncode()
    return result

run('fresh-native-consumer', [sys.executable, '-m', 'venv', consumer])
run('ordinary-review-Git-pip', [python, '-m', 'pip', 'install', 'git+https://github.com/StableKite/SKLint.git@' + REF])
run('pip-check', [python, '-m', 'pip', 'check'])
audit = '''from pathlib import Path
from hashlib import sha256
import importlib.metadata as m,json,base64,sys,sklint
d=m.distribution('sklint');direct=json.loads(d.read_text('direct_url.json'))
assert d.version==sklint.__version__=='0.1.54'
assert direct['url']=='https://github.com/StableKite/SKLint.git'
assert direct['vcs_info']=={'vcs':'git','requested_revision':sys.argv[2],'commit_id':sys.argv[1]}
assert sys.flags.gil is None and not sys._is_gil_enabled()
count=0;payloads=[]
for f in d.files:
 if f.hash is not None:
  data=d.locate_file(f).read_bytes();assert f.hash.mode=='sha256' and f.size==len(data)
  assert base64.urlsafe_b64encode(sha256(data).digest()).rstrip(b'=').decode()==f.hash.value;count+=1
 name=str(f).replace('\\\\','/')
 if name.startswith('sklint/') and name.endswith('.py'):
  actual=d.locate_file(f).read_bytes();expected=(Path(sys.argv[3])/'python'/name).read_bytes()
  assert actual==expected;payloads.append({'path':name,'sha256':sha256(actual).hexdigest()})
assert count==14 and len(payloads)>=2
print(json.dumps({'direct_url':direct,'RECORD_hashes_verified':count,'all_Python_payloads':payloads,'runtime':sys.version,'GIL':False,'gil_flag':None}))
'''
integrity = json.loads(run('exact-installed-source-and-RECORD', [python, '-I', '-B', '-c', audit, HEAD, REF, source]).stdout)
run('actual-native-version', [python, '-m', 'sklint', '--version', '--verbose'])
run('original-package-smoke', [python, '-I', '-B', source / '.github/scripts/ci_sklint.py', 'smoke'])
cases = root / 'own-regressions'
cases.mkdir()
property_case = '''class Parent:
    def __init__(self, failure: OSError | None) -> None:
        self._failure = failure
    @property
    def pending(self) -> OSError | None:
        return self._failure
class Child(Parent):
    def execute(self) -> None:
        """Execute.

        Raises:
            OSError: If the operation failed.
        """
        if self.pending is not None:
            raise self.pending
'''
parameter_case = '''def execute(failure: OSError) -> None:
    """Execute.

    Raises:
        OSError: If the operation failed.
    """
    raise failure
'''
controls = []
for name, text, expected in [('property-true', property_case, 0), ('property-wrong', property_case.replace('OSError: If', 'ValueError: If'), 1), ('parameter-true', parameter_case, 0)]:
    path = cases / (name + '.py')
    path.write_text(text)
    result = run(name, [python, '-m', 'sklint', 'check', '--format', 'json', path], cwd=cases, required=False)
    diagnostics = json.loads(result.stdout)['diagnostics']
    raised_value_errors = [d for d in diagnostics if d['code'] == 'SKD503']
    assert len(raised_value_errors) == expected
    controls.append({'name': name, 'source_sha256': sha256(path.read_bytes()).hexdigest(), 'SKD503': raised_value_errors, 'all_diagnostics_retained': diagnostics})
result = {'status': 'PASS_ORDINARY_NATIVE_LINUX_GIT_INSTALL_ORIGINAL_SMOKE_AND_OWN_PUBLIC_REGRESSIONS', 'head': HEAD, 'review_ref': REF, 'integrity': integrity, 'controls': controls, 'stages': rows, 'Root_source_or_config_uploaded': False, 'R68_gate_replaced': False, 'default_SDK_or_index_changed': False}
(out / 'RESULT.json').write_text(json.dumps(result, indent=2) + '\n')
