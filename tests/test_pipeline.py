"""Tests run against actual Rust CLI, ephemeral real Git trees and SQLite files."""
from __future__ import annotations
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / 'core/target/debug/relgit'


def module(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools' / file)
    assert spec and spec.loader
    obj = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj

pipeline = module('relgit_pipeline','pipeline.py')
protocol = module('relgit_protocol','protocol.py')
comparison = module('relgit_compare','compare.py')


def cmd(*args: str, cwd: Path | None = None, ok: bool=True) -> subprocess.CompletedProcess[str]:
    p = subprocess.run(args, cwd=cwd, capture_output=True,text=True,check=False)
    if ok and p.returncode:
        raise AssertionError(f'{args}: {p.returncode} {p.stderr}')
    return p


class ScannerIntegration(unittest.TestCase):
    def setUp(self):
        if not BIN.is_file(): self.skipTest('build Rust binary first (cargo build)')
        self.tmp = tempfile.TemporaryDirectory(prefix='relgit-security-test-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root/'project'
        self.repo.mkdir()
        cmd('git','init','-q','-b','main',str(self.repo))
        cmd('git','-C',str(self.repo),'config','user.name','Test')
        cmd('git','-C',str(self.repo),'config','user.email','relgit-test@example.invalid')
        (self.repo/'app.go').write_text('package app\nfunc Process(x string) string {\n return x\n}\n')
        self.commit()
        self.report = self.root/'report.json'
        self.db = self.root/'findings.sqlite'

    def commit(self):
        cmd('git','-C',str(self.repo),'add','-A')
        cmd('git','-C',str(self.repo),'commit','-q','-m','source snapshot')

    def codeql(self, line: int=2):
        return {'version': '2.1.0', '$schema':'https://json.schemastore.org/sarif-2.1.0.json', 'runs':[{
            'tool':{'driver':{'name':'CodeQL','version':'2.22.0'}},
            'results':[{'ruleId':'go/path-injection','message':{'text':'candidate only'},
                        'locations':[{'physicalLocation': {'artifactLocation':{'uri':'app.go'},
                                                            'region':{'startLine':line}}}]}]}]}

    def semgrep(self):
        return {'version':'1.0.0','results':[{'check_id':'test.rule','path':'./app.go','start':{'line':3},
                                            'extra':{'message':'bounded candidate','severity':'WARNING'}}], 'errors':[]}

    def ingest(self,engine:str='codeql',audit:bool=False):
        return cmd(str(BIN), 'security-ingest-audit' if audit else 'security-ingest',
                   str(self.repo), str(self.report), str(self.db), engine, ok=False)

    def test_codeql_roundtrip_and_unverified_status(self):
        self.report.write_text(json.dumps(self.codeql()))
        a=self.ingest(); b=self.ingest(audit=True)
        self.assertEqual(a.returncode,0,a.stderr)
        self.assertEqual(b.returncode,0,b.stderr)
        first=json.loads(a.stdout); replay=json.loads(b.stdout)
        self.assertEqual(first['run_id'],replay['run_id'])
        self.assertEqual(replay['candidate_count'],1)
        self.assertEqual(replay['status'],'UNVERIFIED_STATIC_CANDIDATES')
        self.assertTrue(replay['independently_replayed'])
        self.assertEqual(len(comparison.get(self.db,first['run_id'])),1)

    def test_semgrep_roundtrip(self):
        self.report.write_text(json.dumps(self.semgrep()))
        self.assertEqual(self.ingest('semgrep').returncode,0)
        self.assertEqual(self.ingest('semgrep',audit=True).returncode,0)

    def test_mutated_sql_is_refused(self):
        self.report.write_text(json.dumps(self.semgrep()))
        self.assertEqual(self.ingest('semgrep').returncode,0)
        with sqlite3.connect(self.db) as conn:
            conn.execute('UPDATE security_ingest_candidate SET rule_id=?',('forged',))
        self.assertNotEqual(self.ingest('semgrep',audit=True).returncode,0)

    def test_missing_sql_is_refused(self):
        self.report.write_text(json.dumps(self.codeql()))
        self.assertEqual(self.ingest().returncode,0)
        with sqlite3.connect(self.db) as conn:
            conn.execute('DELETE FROM security_ingest_candidate')
        self.assertNotEqual(self.ingest(audit=True).returncode,0)

    def test_source_mutation_is_refused(self):
        self.report.write_text(json.dumps(self.semgrep()))
        self.assertEqual(self.ingest('semgrep').returncode,0)
        (self.repo/'app.go').write_text('package app\nfunc Evil() {}\n')
        self.assertNotEqual(self.ingest('semgrep',audit=True).returncode,0)

    def test_untracked_source_is_refused(self):
        self.report.write_text(json.dumps(self.semgrep()))
        (self.repo/'something.go').write_text('package app\n')
        self.assertNotEqual(self.ingest('semgrep').returncode,0)

    def test_absolute_and_escaped_uri_are_refused(self):
        for uri in ['/etc/passwd','../outside.go','file:///app.go','app%2Ego']:
            value=self.codeql()
            value['runs'][0]['results'][0]['locations'][0]['physicalLocation']['artifactLocation']['uri']=uri
            self.report.write_text(json.dumps(value))
            self.assertNotEqual(self.ingest().returncode,0,uri)

    def test_invalid_line_is_refused(self):
        self.report.write_text(json.dumps(self.codeql(999)))
        self.assertNotEqual(self.ingest().returncode,0)

    def test_semgrep_parser_errors_refuse_success(self):
        report=self.semgrep()
        report['errors']=[{'message':'failed to parse target'}]
        self.report.write_text(json.dumps(report))
        self.assertNotEqual(self.ingest('semgrep').returncode,0)

    def test_failed_codeql_invocation_refuse_success(self):
        report=self.codeql()
        report['runs'][0]['invocations']=[{'executionSuccessful':False}]
        self.report.write_text(json.dumps(report))
        self.assertNotEqual(self.ingest().returncode,0)

    def test_two_separate_revisions_in_same_database(self):
        self.report.write_text(json.dumps(self.semgrep()))
        before=json.loads(self.ingest('semgrep').stdout)
        (self.repo/'app.go').write_text('package app\nfunc Process(x string) string {\n return x + "a"\n}\n')
        self.commit()
        second=self.root/'second.json'
        second.write_text(json.dumps({'version':'1.0','results':[], 'errors':[]}))
        after=cmd(str(BIN),'security-ingest',str(self.repo),str(second),str(self.db),'semgrep',ok=False)
        self.assertEqual(after.returncode,0,after.stderr)
        self.assertEqual(json.loads(after.stdout)['candidate_count'],0)
        audit=cmd(str(BIN),'security-ingest-audit',str(self.repo),str(second),str(self.db),'semgrep',ok=False)
        self.assertEqual(audit.returncode,0,audit.stderr)
        # A missing static warning on the second revision must not imply a security fix.
        left=comparison.get(self.db,before['run_id'])
        right=comparison.get(self.db,json.loads(after.stdout)['run_id'])
        self.assertEqual(len(comparison.compare(left,right)['disappeared']),1)

    def test_wrong_tool_label_is_refused(self):
        x=self.codeql()
        x['runs'][0]['tool']['driver']['name']='Other'
        self.report.write_text(json.dumps(x))
        self.assertNotEqual(self.ingest().returncode,0)

    def test_duplicate_json_object_keys_are_refused(self):
        self.report.write_text('{"version":"2.1.0","version":"2.1.0","runs":[]}')
        self.assertNotEqual(self.ingest().returncode,0)

    def test_protocol_freeze(self):
        dest=self.root/'frozen.json'
        digest=protocol.freeze([self.repo],['authorization','parsing'],dest)
        self.assertEqual(len(digest),64)
        self.assertNotIn('CVE',dest.read_text())
        with self.assertRaises(FileExistsError):
            protocol.freeze([self.repo],['authorization'],dest)

    def test_pipeline_scan_with_controlled_scanner(self):
        fake=self.root/'fake-semgrep'
        fake.write_text("#!/usr/bin/env python3\nimport json, sys\nfrom pathlib import Path\nassert '--metrics=off' in sys.argv and '--json' in sys.argv\nassert sys.argv[-1] == '.'\nout=Path(sys.argv[sys.argv.index('--output')+1])\nout.write_text(json.dumps({'results':[{'check_id':'local.rule','path':'app.go','start':{'line':2},'extra':{'message':'a candidate'}}],'errors':[]}))\n")
        fake.chmod(0o755)
        rules=self.root/'rules.yaml'
        rules.write_text('rules: []\n')
        output=cmd(sys.executable,str(ROOT/'tools/pipeline.py'),'scan','--engine','semgrep',
                   '--repo',str(self.repo),'--report',str(self.report),'--evidence',str(self.db),
                   '--config',str(rules),'--executable',str(fake),'--rust',str(BIN))
        self.assertEqual(json.loads(output.stdout)['receipt']['candidate_count'],1)

    def test_pipeline_import(self):
        self.report.write_text(json.dumps(self.semgrep()))
        response=cmd(sys.executable, str(ROOT/'tools/pipeline.py'),'import','--engine','semgrep',
                     '--repo',str(self.repo),'--report',str(self.report),
                     '--evidence',str(self.db),'--rust',str(BIN))
        r=json.loads(response.stdout)
        self.assertEqual(r['receipt']['candidate_count'],1)
        self.assertTrue(r['receipt']['independently_replayed'])

    def test_pipeline_does_not_allow_evidence_under_repo(self):
        self.report.write_text(json.dumps(self.semgrep()))
        p=cmd(sys.executable,str(ROOT/'tools/pipeline.py'),'import','--engine','semgrep',
              '--repo',str(self.repo),'--report',str(self.report),
              '--evidence',str(self.repo/'bad.sqlite'),'--rust',str(BIN),ok=False)
        self.assertNotEqual(p.returncode,0)

    def test_no_silent_security_fix_from_report_disappearance(self):
        before={'abc':{'path':'app.go'}}
        after={}
        difference=comparison.compare(before,after)
        self.assertIn('abc',difference['disappeared'])
        self.assertEqual(difference['status'],'DIFF_ONLY_NOT_SECURITY_CONFIRMATION')


if __name__=='__main__': unittest.main()
