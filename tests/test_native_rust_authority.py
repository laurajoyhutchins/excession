"""Regression tests for Rust-owned historical native dependency declarations.\nRust symbols are delegated to rust-analyzer SCIP.

Only language resolution occurs outside Rust. The public declaration and
stats must not depend on worktree contents or untracked files.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import excession_native as native


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.PIPE).decode().strip()


class RustNativeAuthorityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / 'repo'
        self.repo.mkdir()
        git(self.repo, 'init', '-q')
        git(self.repo, 'config', 'user.name', 'Test')
        git(self.repo, 'config', 'user.email', 'test@example.org')
        try:
            self.binary = native.binary()
        except native.Unsupported as error:
            self.fail(f'Rust CLI is required for this regression: {error}')

    def commit(self, files):
        for name, contents in files.items():
            filename = self.repo / name
            filename.parent.mkdir(parents=True, exist_ok=True)
            filename.write_text(contents)
        git(self.repo, 'add', '-A')
        git(self.repo, 'commit', '-qm', 'fixture')
        return git(self.repo, 'rev-parse', 'HEAD')

    def paths(self, result):
        return {e['path'] for e in result['inputs']}

    def test_go_historical_and_worktree_independent(self):
        if not shutil.which('go'):
            self.skipTest('Go compiler unavailable')
        old = self.commit({
            'go.mod': 'module example.org/test\n\ngo 1.23\n',
            'main.go': 'package test\nimport "example.org/test/dep"\nvar X=dep.X\n',
            'dep/dep.go': 'package dep\nvar X=1\n',
            'other/other.go': 'package other\n',
        })
        declaration, first_stats = native.discover(self.repo, old, 'main.go', 'go')
        self.assertEqual(self.paths(declaration), {'go.mod', 'main.go', 'dep/dep.go'})
        self.assertEqual(first_stats['commit'], old)
        self.assertEqual(declaration['coverage'], 'DECLARED_INPUTS_ONLY')
        # Mutate checkout without committing: the pinned Git state is unchanged.
        (self.repo / 'dep/dep.go').write_text('package dep\nvar X=999\n')
        (self.repo / 'untracked.go').write_text('package test\n')
        again, stats = native.discover(self.repo, old, 'main.go', 'go')
        self.assertEqual(again, declaration)
        self.assertEqual(stats['commit'], old)

    def test_typescript_alias_and_dynamic_refusal(self):
        if not shutil.which('node') or not shutil.which('tsc'):
            self.skipTest('TypeScript native compiler unavailable')
        old = self.commit({
            'tsconfig.json': json.dumps({'compilerOptions': {
                'baseUrl': '.', 'paths': {'@/*': ['lib/*']}}}),
            'src/main.ts': 'import {x} from "@/math"; console.log(x);\n',
            'lib/math.ts': 'export const x = 1;\n',
            'other.ts': 'export const y = 2;\n',
        })
        try:
            declaration, _ = native.discover(self.repo, old, 'src/main.ts', 'typescript')
        except native.Unsupported as error:
            if 'TypeScript compiler API missing beside tsc' in str(error):
                self.skipTest('TypeScript compiler API unavailable: explicit UNSUPPORTED')
            raise
        self.assertEqual(self.paths(declaration),
                         {'tsconfig.json', 'src/main.ts', 'lib/math.ts'})
        dynamic = self.commit({'src/main.ts': 'require(dynamicName());\n'})
        with self.assertRaises(native.Unsupported):
            native.discover(self.repo, dynamic, 'src/main.ts', 'typescript')

    def test_rust_native_depfiles_are_not_current_evidence_authority(self):
        rev = self.commit({'src/lib.rs': 'fn main() {}'})
        with self.assertRaisesRegex(native.Unsupported, 'rust-analyzer SCIP'):
            native.discover(self.repo, rev, 'src/lib.rs', 'rust')

    def test_trust_opt_in_required_in_rust_and_python(self):
        old = self.commit({'main.js': 'console.log(1);\n'})
        raw = subprocess.run([str(self.binary), 'evidence', 'discover-native',
              '--repo', str(self.repo), '--rev', old, '--entry', 'main.js',
              '--language', 'javascript', '--trusted-local', 'false'], capture_output=True)
        self.assertEqual(raw.returncode, 2)
        self.assertIn(b'trusted-local', raw.stderr)
        wrapper = subprocess.run([sys.executable, str(ROOT/'tools/excession_native.py'),
              str(self.repo), 'main.js', '--revision', old, '--language', 'javascript'], capture_output=True)
        self.assertEqual(wrapper.returncode, 2)
        self.assertIn(b'trusted-local', wrapper.stderr)


if __name__ == '__main__':
    unittest.main()
