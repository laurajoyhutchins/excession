"""Differential acceptance of the Rust-owned slice implementation.

Only the CPython AST import analyzer remains Python-native. The assertions
independently construct expected Git-backed manifests and receipts.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('EXCESSION_BIN', ROOT / 'core/target/debug/excession'))
sys.path.insert(0, str(ROOT / 'tools'))
import excession_slice as slice_client


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], text=True).strip()


def identity(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()).hexdigest()


@unittest.skipUnless(BIN.is_file(), 'build Rust CLI before running integration')
class RustSliceAuthority(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix='excession-rust-slice-')
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        git(self.repo, 'init', '-q')
        git(self.repo, 'config', 'user.email', 'slice@example.invalid')
        git(self.repo, 'config', 'user.name', 'Test')
        self.write('pkg/__init__.py', '')
        self.write('pkg/value.py', 'def number(): return 7\n')
        self.write('tests/__init__.py', '')
        self.write('tests/check.py', 'from pkg.value import number\nassert number() == 7\nprint("yes")\n')
        self.write('unrelated.py', 'this was never valid Python !!!\n')
        self.commit('base')
        self.base = git(self.repo, 'rev-parse', 'HEAD')

    def write(self, name, value):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)

    def commit(self, message):
        git(self.repo, 'add', '-A')
        git(self.repo, 'commit', '-qm', message)

    def cli(self, *argv):
        return subprocess.run([str(BIN), *map(str, argv)], capture_output=True, text=True, timeout=65)

    def manifest(self):
        return slice_client.plan(self.repo, 'tests/check.py')

    def test_manifest_is_exact_git_object_not_a_python_claim(self):
        observed = self.manifest()
        entries = []
        for path in ['pkg/__init__.py', 'pkg/value.py', 'tests/__init__.py', 'tests/check.py']:
            data = (self.repo / path).read_bytes()
            entries.append({'path': path, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                            'blob': git(self.repo, 'rev-parse', self.base + ':' + path)})
        expected = {'schema': slice_client.SCHEMA, 'commit': self.base,
                    'tree': git(self.repo, 'rev-parse', self.base + '^{tree}'),
                    'entry': 'tests/check.py', 'assets': [], 'entries': entries,
                    'scope': 'STATIC_IMPORT_CLOSURE_ONLY', 'semantic_equivalence': 'UNVERIFIED'}
        expected['view_id'] = identity(expected)
        self.assertEqual(observed, expected)
        self.assertNotIn('unrelated.py', [e['path'] for e in observed['entries']])

    def test_historical_source_and_dirty_worktree_do_not_change_manifest(self):
        manifest = self.manifest()
        self.write('pkg/value.py', 'def number(): return 8\n')
        historical = slice_client.plan(self.repo, 'tests/check.py', self.base, history=True)
        self.assertEqual(historical, manifest)
        with self.assertRaises(slice_client.Refusal):
            slice_client.plan(self.repo, 'tests/check.py')
        self.assertIn('return 8', (self.repo / 'pkg/value.py').read_text())
        directory = self.root / 'historical'
        slice_client.materialize(self.repo, historical, directory)
        self.assertIn('return 7', (directory / 'pkg/value.py').read_text())
        slice_client.audit(self.repo, historical, directory)

    def test_no_silent_execution_without_trust_approval(self):
        manifest = self.manifest()
        path = self.root / 'manifest.json'
        path.write_text(json.dumps(manifest))
        directory = self.root / 'slice'
        slice_client.materialize(self.repo, manifest, directory)
        denied = self.cli('slice', 'run', '--repo', self.repo, '--manifest', path,
                          '--directory', directory, '--trusted-local', 'false')
        self.assertEqual(denied.returncode, 2)
        self.assertIn('--trusted-local', denied.stderr)
        self.assertNotIn('OBSERVED_FAIL', denied.stdout)

    def test_slice_must_be_outside_checkout(self):
        manifest = self.manifest()
        with self.assertRaises(slice_client.Refusal):
            slice_client.materialize(self.repo, manifest, self.repo / 'derived')
        self.assertFalse((self.repo / 'derived').exists())

    def test_forged_source_with_recomputed_view_digest_is_rejected(self):
        manifest = self.manifest()
        manifest['entries'][0]['sha256'] = '0' * 64
        manifest['view_id'] = identity({k: v for k, v in manifest.items() if k != 'view_id'})
        with self.assertRaises(slice_client.Refusal):
            slice_client.validate_manifest(self.repo, manifest)

    def test_receipt_is_canonical_and_replay_is_new_execution(self):
        manifest = self.manifest()
        directory = self.root / 'slice'
        slice_client.materialize(self.repo, manifest, directory)
        receipt = slice_client.execute(self.repo, manifest, directory, 10)
        self.assertEqual(receipt['status'], 'OBSERVED_PASS')
        self.assertEqual(receipt['stdout_sha256'], hashlib.sha256(b'yes\n').hexdigest())
        self.assertEqual(receipt['receipt_id'], identity({k: v for k, v in receipt.items() if k != 'receipt_id'}))
        self.write('notes.md', 'new documentation\n')
        self.commit('nonsemantic')
        result = slice_client.carry(self.repo, manifest, receipt)
        self.assertEqual(result['decision'], 'SOURCE_MATCH_ONLY')
        self.assertFalse(result['target_behavior_verified'])
        m = self.root / 'manifest.json'
        r = self.root / 'receipt.json'
        m.write_text(json.dumps(manifest))
        r.write_text(json.dumps(receipt))
        output = self.cli('slice', 'carry', '--repo', self.repo, '--manifest', m,
                          '--receipt', r, '--rev', 'HEAD', '--replay', 'true',
                          '--trusted-local', 'true')
        self.assertEqual(output.returncode, 0, output.stderr)
        newer = json.loads(output.stdout)
        self.assertEqual(newer['fresh_target_receipt']['status'], 'OBSERVED_PASS')
        self.assertNotEqual(newer['fresh_target_receipt']['commit'], receipt['commit'])
        self.assertFalse(newer['target_behavior_verified'])

    def test_namespace_never_returns_unsandboxed_receipt(self):
        manifest = self.manifest()
        location = self.root / 'slice'
        slice_client.materialize(self.repo, manifest, location)
        m = self.root / 'manifest.json'
        m.write_text(json.dumps(manifest))
        result = self.cli('slice', 'run', '--repo', self.repo, '--manifest', m,
                          '--directory', location, '--trusted-local', 'true',
                          '--isolated-linux', 'true')
        if result.returncode == 0:
            self.assertEqual(json.loads(result.stdout)['isolation'], 'LOCAL_LINUX_NAMESPACES_UNATTESTED')
        else:
            self.assertEqual(result.returncode, 2)
            self.assertIn('REFUSED:', result.stderr)
            self.assertEqual(result.stdout, '')


if __name__ == '__main__':
    unittest.main()
