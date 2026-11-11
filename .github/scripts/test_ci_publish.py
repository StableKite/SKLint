"""Offline publication controls: no production credentials or HTTP requests."""
from pathlib import Path
import base64
import csv
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('sklint_publish', Path(__file__).with_name('ci_publish.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class Reply:
    def __init__(self, status=200, data=None, content='application/json', headers=None):
        self.status_code = status
        self.headers = {'Content-Type': content, **(headers or {})}
        self.data = data

    def json(self):
        return self.data


class Session:
    def __init__(self, reply):
        self.reply = reply
        self.calls = []

    def get(self, *args, **kwargs):
        self.calls.append((args, kwargs))
        return self.reply


class ReleaseControls(unittest.TestCase):
    def test_unchanged_push_skips_publication(self):
        self.assertEqual(release.release_decision('push', 'refs/heads/master', 'master', '1.2', '1.2'),
                         (False, 'unchanged-version'))

    def test_changed_default_push_publishes(self):
        self.assertEqual(release.release_decision('push', 'refs/heads/master', 'master', '1.2', '1.3'),
                         (True, 'version-change'))

    def test_all_publication_phases_refuse_validation_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release.write(root / 'PLAN.json', {'should_publish': False})
            for function, extra in [(release.collision_release, ()), (release.validate_release, (root,)),
                                    (release.upload_release, ()), (release.confirm_release, ())]:
                with self.subTest(phase=function.__name__), self.assertRaisesRegex(ValueError, 'forbidden for validation-only'):
                    function(root, root, *extra)
            self.assertEqual({path.name for path in root.iterdir()}, {'PLAN.json'})

    def test_collision_checks_semantic_order_including_local_versions(self):
        release.check_collision(['1.9', '1.10rc1'], '1.10')
        for versions, candidate in [(['1.10'], '1.10'), (['1.11'], '1.10'),
                                    (['1.10+other'], '1.10'), (['1.10.post1'], '1.10')]:
            with self.subTest(versions=versions), self.assertRaises(ValueError):
                release.check_collision(versions, candidate)

    def test_invalid_server_version_is_not_treated_as_absent(self):
        with self.assertRaises(ValueError):
            release.check_collision(['not-a-version'], '1.10')

    def test_read_phases_reject_write_token_even_with_read_credential(self):
        with patch.dict(os.environ, {'DEVPI_TOKEN': 'offline-placeholder',
                                   'DEVPI_READ_TOKEN': 'offline-placeholder'}, clear=True):
            with self.assertRaisesRegex(ValueError, 'Write-only'):
                release.read_session()

    def test_missing_read_credential_fails_closed(self):
        with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(ValueError, 'separately authorized'):
            release.read_session()

    def test_read_session_tls_and_credential_separation(self):
        with patch.dict(os.environ, {'DEVPI_READ_TOKEN': 'offline-placeholder'}, clear=True):
            with release.read_session() as session:
                self.assertIs(session.verify, True)
                self.assertFalse(session.trust_env)
                self.assertEqual(session.auth, ('', 'offline-placeholder'))

    def test_only_canonical_https_urls_are_accepted(self):
        self.assertEqual(release.safe_url(release.INDEX + '+api'), release.INDEX + '+api')
        for url in ['http://devpi.stablekite.com/StableKite/index/+api',
                    'https://devpi.stablekite.com:4438/StableKite/index/+api',
                    'https://other.example/StableKite/index/+api',
                    'https://devpi.stablekite.com/StableKite/index/../other',
                    'https://devpi.stablekite.com/StableKite/index/%2e%2e/other',
                    'https://devpi.stablekite.com/StableKite/index/file?redirect=other']:
            with self.subTest(url=url), self.assertRaises(ValueError):
                release.safe_url(url)

    def test_redirect_auth_html_and_invalid_schema_fail_closed(self):
        for reply in [Reply(302), Reply(401), Reply(403), Reply(500), Reply(content='text/html'),
                      Reply(data={'type': 'different', 'result': {}})]:
            session = Session(reply)
            with self.subTest(status=reply.status_code), self.assertRaises(ValueError):
                release.get_json(session, release.INDEX + '+api', 'apiconfig')
            self.assertIs(session.calls[0][1]['allow_redirects'], False)
            self.assertEqual(session.calls[0][1]['timeout'], (10, 30))

    def test_only_api_verified_404_means_project_absent(self):
        with self.assertRaises(ValueError):
            release.get_json(Session(Reply(404)), release.INDEX + 'sklint', 'projectconfig', True)
        self.assertEqual(release.get_json(Session(Reply(404, headers={'X-DEVPI-API-VERSION': '2'})),
                         release.INDEX + 'sklint', 'projectconfig', True), {})

    def test_authentication_must_confirm_identity_and_exact_index(self):
        for result in [{'authstatus': ['noauth', '', []], 'index': release.INDEX},
                       {'authstatus': ['ok', 'owner', []], 'index': 'https://other.example/index/'}]:
            with self.subTest(result=result), self.assertRaises(ValueError):
                release.authenticate(Session(Reply(data={'type': 'apiconfig', 'result': result})))
        release.authenticate(Session(Reply(data={'type': 'apiconfig', 'result':
                             {'authstatus': ['ok', 'owner', []], 'index': release.INDEX.rstrip('/')}})))

    def test_cross_platform_manifest_paths_and_escape_rejection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'distributions').mkdir()
            path = root / 'distributions/a.whl'
            path.write_bytes(b'not interpreted as a wheel')
            self.assertEqual(release.safe_file(root, 'distributions\\a.whl'), path.resolve())
            for relative in ['../a.whl', 'C:/a.whl', '/a.whl', 'distributions/../../a.whl']:
                with self.subTest(relative=relative), self.assertRaises(ValueError):
                    release.safe_file(root, relative)

    def test_manual_requires_explicit_input_and_exact_default(self):
        self.assertFalse(release.release_decision('workflow_dispatch', 'refs/heads/main', 'main', None, '1.2')[0])
        self.assertTrue(release.release_decision('workflow_dispatch', 'refs/heads/main', 'main', None, '1.2', True)[0])
        for event, ref in [('push', 'refs/heads/feature'), ('workflow_dispatch', 'refs/heads/feature'),
                           ('push', 'refs/heads/Main')]:
            with self.subTest(event=event, ref=ref):
                self.assertFalse(release.release_decision(event, ref, 'main', '1.1', '1.2', True)[0])
        with self.assertRaises(ValueError):
            release.release_decision('pull_request', 'refs/pull/1/merge', 'main', None, '1.2')

    def test_all_five_original_gates_are_mandatory(self):
        good = {name: {'result': 'success', 'outputs': {}} for name in release.PRODUCERS}
        self.assertEqual(release.successful_producers(good), good)
        for name in release.PRODUCERS:
            for result in ('failure', 'skipped', 'cancelled', None):
                wrong = {**good, name: {'result': result}}
                with self.subTest(name=name, result=result), self.assertRaises(ValueError):
                    release.successful_producers(wrong)
            wrong = good.copy()
            wrong.pop(name)
            with self.assertRaises(ValueError):
                release.successful_producers(wrong)

    def test_previous_version_has_no_guess_or_zero_sha_fallback(self):
        for invalid in ('', '0' * 40, 'z' * 40, 'a' * 39):
            with self.subTest(invalid=invalid), patch.object(release, 'git') as git, self.assertRaises(ValueError):
                release.previous_version(Path('.'), invalid)
            git.assert_not_called()
        with patch.object(release, 'git', side_effect=subprocess.CalledProcessError(1, 'git')):
            with self.assertRaises(subprocess.CalledProcessError):
                release.previous_version(Path('.'), 'a' * 40)
        with patch.object(release, 'git', side_effect=['[project]\nname="sklint"\nversion="1.2"', '__version__ = "1.3"']):
            with self.assertRaisesRegex(ValueError, 'versions differ'):
                release.previous_version(Path('.'), 'a' * 40)

    def test_canonical_source_and_runtime_versions_must_match(self):
        for project, runtime in [('1.2', '1.3'), ('1.2+local', '1.2+local'), ('not-a-version', 'not-a-version')]:
            with self.subTest(project=project), self.assertRaises(ValueError):
                release.version_pair('[project]\nname="sklint"\nversion="' + project + '"',
                                     '__version__ = "' + runtime + '"')

    def test_plan_binds_run_and_unchanged_push_skips_every_network_phase(self):
        head = 'a' * 40
        env = {'GITHUB_SHA': head, 'GITHUB_REPOSITORY': 'StableKite/SKLint', 'GITHUB_REF': 'refs/heads/main',
               'GITHUB_EVENT_NAME': 'push', 'GITHUB_RUN_ID': '100', 'GITHUB_RUN_ATTEMPT': '1',
               'NEEDS_JSON': json.dumps({name: {'result': 'success'} for name in release.PRODUCERS})}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            event = {'repository': {'full_name': 'StableKite/SKLint', 'default_branch': 'main'}, 'before': 'b' * 40}
            release.write(root / 'event.json', event)
            env.update(GITHUB_EVENT_PATH=str(root / 'event.json'), GITHUB_OUTPUT=str(root / 'output.txt'))
            with patch.dict(os.environ, env, clear=True), patch.object(release, 'git', return_value=head), \
                 patch.object(release, 'clean_source'), patch.object(release, 'source_version', return_value='1.2'), \
                 patch.object(release, 'previous_version', return_value='1.2'), patch.object(release, 'runtime_policy', return_value={'status': 'offline'}):
                release.plan_release(root, root)
                plan = release.read(root / 'PLAN.json')
                self.assertEqual(plan['previous_version'], plan['current_version'])
                self.assertFalse(plan['should_publish'])
                for function, extra in [(release.collision_release, ()), (release.validate_release, (root,)),
                                        (release.upload_release, ()), (release.confirm_release, ())]:
                    with self.subTest(phase=function.__name__), patch('requests.Session') as session, self.assertRaises(ValueError):
                        function(root, root, *extra)
                    session.assert_not_called()

    def test_new_nondefault_branch_has_no_previous_commit_and_cannot_publish(self):
        head = 'a' * 40
        env = {'GITHUB_SHA': head, 'GITHUB_REPOSITORY': 'StableKite/SKLint',
               'GITHUB_REF': 'refs/heads/review', 'GITHUB_EVENT_NAME': 'push',
               'GITHUB_RUN_ID': '100', 'GITHUB_RUN_ATTEMPT': '1',
               'NEEDS_JSON': json.dumps({name: {'result': 'success'} for name in release.PRODUCERS})}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release.write(root / 'event.json', {'repository': {'full_name': 'StableKite/SKLint',
                          'default_branch': 'main'}, 'before': '0' * 40})
            env.update(GITHUB_EVENT_PATH=str(root / 'event.json'), GITHUB_OUTPUT=str(root / 'output.txt'))
            with patch.dict(os.environ, env, clear=True), patch.object(release, 'git', return_value=head), \
                 patch.object(release, 'clean_source'), patch.object(release, 'source_version', return_value='1.3'), \
                 patch.object(release, 'runtime_policy', return_value={'status': 'offline'}):
                release.plan_release(root, root)
                plan = release.read(root / 'PLAN.json')
                self.assertIsNone(plan['previous_version'])
                self.assertFalse(plan['should_publish'])
                self.assertEqual(plan['reason'], 'validation-only')
                for function, extra in [(release.collision_release, ()), (release.validate_release, (root,)),
                                        (release.upload_release, ()), (release.confirm_release, ())]:
                    with self.subTest(phase=function.__name__), patch('requests.Session') as session, self.assertRaises(ValueError):
                        function(root, root, *extra)
                    session.assert_not_called()

    def test_new_default_push_still_requires_real_previous_commit(self):
        head = 'a' * 40
        env = {'GITHUB_SHA': head, 'GITHUB_REPOSITORY': 'StableKite/SKLint',
               'GITHUB_REF': 'refs/heads/main', 'GITHUB_EVENT_NAME': 'push',
               'GITHUB_RUN_ID': '100', 'GITHUB_RUN_ATTEMPT': '1',
               'NEEDS_JSON': json.dumps({name: {'result': 'success'} for name in release.PRODUCERS})}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release.write(root / 'event.json', {'repository': {'full_name': 'StableKite/SKLint',
                          'default_branch': 'main'}, 'before': '0' * 40})
            env.update(GITHUB_EVENT_PATH=str(root / 'event.json'), GITHUB_OUTPUT=str(root / 'output.txt'))
            with patch.dict(os.environ, env, clear=True), patch.object(release, 'git', return_value=head), \
                 patch.object(release, 'clean_source'), patch.object(release, 'source_version', return_value='1.3'), \
                 patch.object(release, 'runtime_policy', return_value={'status': 'offline'}), \
                 self.assertRaisesRegex(ValueError, 'A real previous commit is required'):
                release.plan_release(root, root)
            self.assertFalse((root / 'PLAN.json').exists())
            self.assertFalse((root / 'output.txt').exists())

    def test_network_plan_rejects_stale_run_head_metadata_and_modified_source(self):
        head = 'a' * 40
        plan = {'should_publish': True, 'git_head': head, 'repository': 'StableKite/SKLint',
                'run_id': '100', 'run_attempt': '1', 'ref': 'refs/heads/main', 'default_branch': 'main',
                'current_version': '1.2', 'endpoint': release.INDEX,
                'producers': {name: {'result': 'success'} for name in release.PRODUCERS}}
        env = {'GITHUB_SHA': head, 'GITHUB_REPOSITORY': 'StableKite/SKLint', 'GITHUB_REF': 'refs/heads/main',
               'GITHUB_RUN_ID': '100', 'GITHUB_RUN_ATTEMPT': '1'}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for field, value in [('git_head', 'b' * 40), ('run_id', '101'), ('run_attempt', '2'),
                                 ('ref', 'refs/heads/feature'), ('current_version', '1.3'), ('endpoint', 'https://other/')]:
                with self.subTest(field=field), patch.dict(os.environ, env, clear=True), \
                     patch.object(release, 'read', return_value={**plan, field: value}), \
                     patch.object(release, 'git', return_value=head), patch.object(release, 'source_version', return_value='1.2'), \
                     self.assertRaises(ValueError):
                    release.checked_plan(root, root)
            with patch.object(release, 'git', return_value=' M setup.py'), self.assertRaises(ValueError):
                release.clean_source(root)

    def make_wheel(self, path, head='a' * 40, dirty='false', corrupt_record=False):
        info = {'version': '1.2', 'revision': 'v1.2', 'source_commit': head, 'source_dirty': dirty,
                'rustc': release.RUSTC, 'target': release.RUST_TARGETS['glibc-x86_64'],
                'wheel_platform_tag': release.PLATFORMS['glibc-x86_64'], 'source_tree_sha256': 'c' * 64}
        prefix = 'sklint-1.2.data/purelib/sklint/bin/'
        files = {'sklint-1.2.dist-info/METADATA': b'Metadata-Version: 2.4\nName: sklint\nVersion: 1.2\n',
                 'sklint-1.2.dist-info/WHEEL': b'Wheel-Version: 1.0\nTag: py3-none-manylinux_2_35_x86_64\n',
                 prefix + 'BUILD_INFO.txt': ''.join(key + '=' + value + '\n' for key, value in info.items()).encode(),
                 prefix + 'TARGET': release.RUST_TARGETS['glibc-x86_64'].encode(), prefix + 'sklint': b'not executed'}
        rows = []
        for name, value in files.items():
            digest = base64.urlsafe_b64encode(hashlib.sha256(value).digest()).rstrip(b'=').decode()
            rows.append([name, 'sha256=' + ('wrong' if corrupt_record else digest), str(len(value))])
        record = 'sklint-1.2.dist-info/RECORD'
        rows.append([record, '', ''])
        stream = io.StringIO()
        csv.writer(stream).writerows(rows)
        files[record] = stream.getvalue().encode()
        with zipfile.ZipFile(path, 'w') as archive:
            for name, value in files.items():
                archive.writestr(name, value)

    def test_wheel_checks_record_and_clean_exact_native_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'sklint-1.2-py3-none-manylinux_2_35_x86_64.whl'
            self.make_wheel(path)
            record = release.check_wheel(path, '1.2', 'glibc-x86_64', 'a' * 40)
            self.assertEqual(record['native_sha256'], hashlib.sha256(b'not executed').hexdigest())
            for change in ({'head': 'b' * 40}, {'dirty': 'true'}, {'corrupt_record': True}):
                self.make_wheel(path, **change)
                with self.subTest(change=change), self.assertRaises(ValueError):
                    release.check_wheel(path, '1.2', 'glibc-x86_64', 'a' * 40)

    def test_missing_extra_or_directory_artifacts_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(ValueError, 'six-artifact'):
                release.collect_artifacts(root, root, '1.2', 'a' * 40)
            for tag in release.PLATFORMS.values():
                (root / ('sklint-1.2-py3-none-' + tag + '.whl')).write_bytes(b'not executed')
            (root / 'sklint-1.2.tar.gz').mkdir()
            with self.assertRaisesRegex(ValueError, 'six-artifact'):
                release.collect_artifacts(root, root, '1.2', 'a' * 40)


if __name__ == '__main__':
    unittest.main(verbosity=2)
