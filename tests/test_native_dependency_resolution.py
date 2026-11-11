"""Native-resolver tests use tiny, real Git histories and pinned compiler tools."""
import importlib.util
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


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.PIPE).decode().strip()


def put(repo, changes, message):
    for name, content in changes.items():
        dest = repo / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(content)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-m', message)
    return git(repo, 'rev-parse', 'HEAD')


class NativeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / 'repo'
        self.repo.mkdir()
        git(self.repo, 'init', '-q')
        git(self.repo, 'config', 'user.email', 'test@example.com')
        git(self.repo, 'config', 'user.name', 'Test')

    def discover(self, sha, entry, language):
        try:
            result, _stats = native.discover(self.repo, sha, entry, language)
        except native.Unsupported as error:
            if language in ('javascript', 'typescript') and (
                    'TypeScript compiler API missing beside tsc' in str(error)):
                self.skipTest('TypeScript compiler API unavailable: explicit UNSUPPORTED')
            raise
        return result

    def test_go_native_transitive_and_build_input_invalidation(self):
        if not shutil.which('go'):
            self.skipTest('go missing')
        base = put(self.repo, {'go.mod': 'module example.org/test\n\ngo 1.23\n',
            'pkg/main.go': 'package pkg\nimport "example.org/test/lib"\nfunc A() int { return lib.X() }\n',
            'pkg/main_test.go': 'package pkg\nimport "testing"\nfunc TestA(t *testing.T) {}\n',
            'lib/lib.go': 'package lib\nfunc X() int { return 1 }\n',
            'other/other.go': 'package other\n'}, 'base')
        first = self.discover(base, 'pkg/main_test.go', 'go')
        self.assertEqual({x['path'] for x in first['inputs']},
                         {'go.mod','pkg/main.go','pkg/main_test.go','lib/lib.go'})
        later = put(self.repo, {'other/other.go': 'package other\n// changed\n'}, 'unrelated')
        same = self.discover(later, 'pkg/main_test.go', 'go')
        self.assertEqual(first['inputs'], same['inputs'])
        changed = put(self.repo, {'lib/lib.go': 'package lib\nfunc X() int { return 2 }\n'}, 'dependent')
        newest = self.discover(changed, 'pkg/main_test.go', 'go')
        self.assertEqual(first['inputs'], newest['inputs'])  # Rust compares Git blob identities
        self.assertEqual(first['assumptions'], newest['assumptions'])
        self.assertNotEqual(native.Tree(self.repo,base).read('lib/lib.go'),
                            native.Tree(self.repo,changed).read('lib/lib.go'))

    def test_ts_compiler_resolves_path_alias_and_cjs(self):
        if not shutil.which('node') or not shutil.which('tsc'):
            self.skipTest('Node/TypeScript missing')
        rev = put(self.repo, {'tsconfig.json': json.dumps({'compilerOptions':{
                            'baseUrl':'.','paths':{'@/*':['lib/*']}}}),
            'src/main.ts': 'import {x} from "@/math"; console.log(x);\n',
            'lib/math.ts': 'export const x = 1;\n',
            'ignore.ts': 'export const y = 2;\n'}, 'typescript alias')
        decl = self.discover(rev,'src/main.ts','typescript')
        self.assertEqual({x['path'] for x in decl['inputs']},
                         {'tsconfig.json','src/main.ts','lib/math.ts'})
        rev = put(self.repo, {'src/main.ts': 'const x = require(getModuleName());\n'}, 'dynamic')
        with self.assertRaises(native.Unsupported):
            self.discover(rev,'src/main.ts','typescript')

    def test_native_rust_depfile_resolver_is_explicitly_retired(self):
        rev = put(self.repo, {'src/lib.rs': 'fn main() {}'}, 'rust source')
        with self.assertRaisesRegex(native.Unsupported, 'rust-analyzer SCIP'):
            self.discover(rev, 'src/lib.rs', 'rust')

    def test_explicit_trust_and_untracked_files(self):
        rev = put(self.repo, {'main.js':'console.log("hello");\n'}, 'base')
        script = str(ROOT/'tools/excession_native.py')
        denied = subprocess.run([sys.executable,script,str(self.repo),'main.js',
                                 '--revision',rev,'--language','javascript'], capture_output=True)
        self.assertEqual(denied.returncode,2)
        self.assertIn(b'--trusted-local',denied.stderr)
        (self.repo/'untracked.js').write_text('console.log(1);')
        if shutil.which('node') and shutil.which('tsc'):
            decl=self.discover(rev,'main.js','javascript')
            self.assertEqual([x['path'] for x in decl['inputs']],['main.js'])

    def test_native_go_declarations_feed_rust_transport_and_audit(self):
        exe=ROOT/'core/target/debug/excession'
        if not exe.is_file() or not shutil.which('go'):
            self.skipTest('native Rust CLI or Go toolchain missing')
        base=put(self.repo, {'go.mod':'module example.org/check\n\ngo 1.23\n',
            'main.go':'package check\nimport "example.org/check/dep"\nvar X=dep.X\n',
            'dep/dep.go':'package dep\nvar X=1\n',
            'unrelated.txt':'a\n'}, 'base')
        unrelated=put(self.repo,{'unrelated.txt':'b\n'},'unrelated')
        modified=put(self.repo,{'dep/dep.go':'package dep\nvar X=2\n'},'dependency')
        contracts=[]
        for commit in (base,unrelated,modified):
            declaration=self.discover(commit,'main.go','go')
            request=Path(self.temp.name)/(commit[:8]+'.decl.json')
            request.write_text(json.dumps(declaration))
            result=subprocess.check_output([str(exe),'evidence','contract-plan','--repo',
                    str(self.repo),'--rev',commit,'--declaration',str(request)])
            contract=Path(self.temp.name)/(commit[:8]+'.contract.json')
            contract.write_bytes(result)
            contracts.append(contract)
        database=Path(self.temp.name)/'evidence.sqlite'
        for target,expected in [(contracts[1],'DECLARED_INPUTS_MATCH_ONLY'),
                                (contracts[2],'REVERIFY_REQUIRED')]:
            args=['--repo',str(self.repo),'--source-contract',str(contracts[0]),
                  '--target-contract',str(target),'--db',str(database)]
            stored=json.loads(subprocess.check_output([str(exe),'evidence',
                                                       'contract-compare',*args]))
            self.assertEqual(stored['decision'],expected)
            audited=json.loads(subprocess.check_output([str(exe),'evidence',
                                                        'contract-audit',*args]))
            self.assertTrue(audited['independently_audited'])
            self.assertFalse(audited['target_behavior_verified'])

    def test_real_postcss_bundle_with_vendored_modules(self):
        if not shutil.which('node') or not shutil.which('tsc'):
            self.skipTest('Node/TypeScript missing')
        bundle=ROOT/'core/fixtures/postcss-crossrepo.bundle'
        if not bundle.is_file():
            self.skipTest('real PostCSS fixture missing')
        target=Path(self.temp.name)/'postcss'
        subprocess.run(['git','clone','-q',str(bundle),str(target)],
                       check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        try:
            decl, stats = native.discover(
                target, 'HEAD', 'postcss/lib/postcss.js', 'javascript')
        except native.Unsupported as error:
            if 'TypeScript compiler API missing beside tsc' in str(error):
                self.skipTest('TypeScript compiler API unavailable: explicit UNSUPPORTED')
            raise
        paths={x['path'] for x in decl['inputs']}
        self.assertIn('postcss/node_modules/nanoid/non-secure/index.cjs',paths)
        self.assertIn('postcss/node_modules/picocolors/picocolors.js',paths)
        self.assertIn('postcss/lib/postcss.js',paths)
        self.assertGreaterEqual(stats['declared_files'],35)


if __name__ == '__main__':
    unittest.main()
