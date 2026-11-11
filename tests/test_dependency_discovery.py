import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('excession_discover',ROOT/'tools/excession_discover.py')
mod=importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

def cmd(repo,*args):
    subprocess.run(['git','-C',str(repo),*args],check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)

def commit(repo,files,msg):
    for path,body in files.items():
        dest=repo/path;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_text(body)
    cmd(repo,'add','-A');cmd(repo,'commit','-m',msg)
    return subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()

class DiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.repo=Path(self.tmp.name)
        cmd(self.repo,'init','-q');cmd(self.repo,'config','user.email','test@example.com');cmd(self.repo,'config','user.name','Test')
    def paths(self,rev,entry,lang):
        return {x['path'] for x in mod.discover(self.repo,rev,entry,lang)['inputs']}
    def test_javascript_and_ts_historical_import_resolution(self):
        a=commit(self.repo,{'test.js':'import {x} from "./src/dep.js"; console.log(x);\n','src/dep.js':'export const x=1;\n','other.js':'const unrelated=1;\n','package.json':'{"private":true}\n'},'base')
        b=commit(self.repo,{'other.js':'const unrelated=2;\n'},'unrelated')
        self.assertEqual(self.paths(a,'test.js','javascript'),self.paths(b,'test.js','javascript'))
        c=commit(self.repo,{'src/dep.js':'export const x=2;\n'},'dependency change')
        self.assertEqual(self.paths(c,'test.js','javascript'),self.paths(b,'test.js','javascript'))
        # The contract uses Git identities to discover the same path with changed bytes.
        self.assertNotEqual(mod.Tree(self.repo,c).read('src/dep.js'),mod.Tree(self.repo,b).read('src/dep.js'))
        commit(self.repo,{'test.ts':'import {f} from "./src/math"; f();\n','src/math.ts':'export const f=()=>1;\n','tsconfig.json':'{"compilerOptions":{"strict":true}}\n'},'TS')
        self.assertIn('src/math.ts',self.paths('HEAD','test.ts','typescript'))
        commit(self.repo,{'test.ts':'import x from "lodash";\n'},'external')
        with self.assertRaises(mod.Unsupported):self.paths('HEAD','test.ts','typescript')
    def test_go_same_package_and_internal_import(self):
        a=commit(self.repo,{'go.mod':'module example.com/local\n\ngo 1.22\n','pkg/check_test.go':'package pkg\nimport "example.com/local/util"\nfunc TestX() {}\n','pkg/check.go':'package pkg\n','util/util.go':'package util\n','other/other.go':'package other\n'},'base')
        result=self.paths(a,'pkg/check_test.go','go')
        self.assertEqual(result,{'go.mod','pkg/check.go','pkg/check_test.go','util/util.go'})
        commit(self.repo,{'pkg/check.go':'package pkg\n// local update\n'},'update')
        self.assertEqual(result,self.paths('HEAD','pkg/check_test.go','go'))
        commit(self.repo,{'util/util.go':'//go:build linux\npackage util\n'},'unsupported build tag')
        with self.assertRaises(mod.Unsupported):self.paths('HEAD','pkg/check_test.go','go')
    def test_rust_conservative_crate_and_include_refusal(self):
        a=commit(self.repo,{'Cargo.toml':'[package]\nname="test"\nversion="0.1.0"\n','src/main.rs':'mod other; fn main() {}\n','src/other.rs':'pub fn f() {}\n','README.md':'hello\n'},'base')
        self.assertEqual(self.paths(a,'src/main.rs','rust'),{'Cargo.toml','src/main.rs','src/other.rs'})
        commit(self.repo,{'src/main.rs':'include_str!("data.txt"); fn main() {}\n'},'include')
        with self.assertRaises(mod.Unsupported):self.paths('HEAD','src/main.rs','rust')
    def test_discovered_contract_transport_end_to_end(self):
        import os
        root=ROOT
        exe=root/'core/target/debug/excession'
        if not exe.is_file():self.skipTest('Rust binary not built')
        first=commit(self.repo,{'main.js':'import {x} from "./dep.js"; console.log(x);\n',
            'dep.js':'export const x=1;\n','other.js':'const n=1;\n','package.json':'{"private":true}\n'},'base')
        second=commit(self.repo,{'other.js':'const n=2;\n'},'unrelated')
        third=commit(self.repo,{'dep.js':'export const x=2;\n'},'dependent')
        manifests=[]
        for revision in (first,second,third):
            declaration=mod.discover(self.repo,revision,'main.js','javascript')
            declaration_path=self.repo/f'decl-{revision[:8]}.json'
            declaration_path.write_text(json.dumps(declaration))
            output=subprocess.check_output([str(exe),'evidence','contract-plan',
                '--repo',str(self.repo),'--rev',revision,'--declaration',str(declaration_path)])
            contract=self.repo/f'contract-{revision[:8]}.json';contract.write_bytes(output)
            manifests.append(contract)
        db=self.repo/'evidence.sqlite'
        for target,expected in ((manifests[1],'DECLARED_INPUTS_MATCH_ONLY'),
                                (manifests[2],'REVERIFY_REQUIRED')):
            args=['--repo',str(self.repo),'--source-contract',str(manifests[0]),
                  '--target-contract',str(target),'--db',str(db)]
            result=json.loads(subprocess.check_output([str(exe),'evidence','contract-compare',*args]))
            self.assertEqual(result['decision'],expected)
            verified=json.loads(subprocess.check_output([str(exe),'evidence','contract-audit',*args]))
            self.assertTrue(verified['independently_audited'])
            self.assertFalse(verified['target_behavior_verified'])

    def test_forged_dynamic_or_symlink_refused(self):
        commit(self.repo,{'main.js':'const x = require(name);\n'},'dynamic')
        with self.assertRaises(mod.Unsupported):self.paths('HEAD','main.js','javascript')
        commit(self.repo,{'main.js':'import "./linked.js";\n'},'relative')
        (self.repo/'linked.js').symlink_to('main.js');cmd(self.repo,'add','-A');cmd(self.repo,'commit','-m','symlink')
        with self.assertRaises(mod.Unsupported):self.paths('HEAD','main.js','javascript')

if __name__=='__main__':unittest.main()
