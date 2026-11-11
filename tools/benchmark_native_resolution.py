#!/usr/bin/env python3
"""Observable dependency resolver comparisons, no behavioral claims.

Run on an actual Git source tree. The three candidate sets differ in scope:
- static: paths from a narrow conservative parser (may refuse)
- native: paths from installed compiler / language-native resolution
- graph: Excession's existing indexed dependency *view* (JS/TS only)
Graph omissions and native omissions are reported, never treated as proofs.
"""
import argparse
import json
import statistics
import subprocess
import tempfile
import time
from pathlib import Path
from excession_discover import discover as static_discover, Unsupported
from excession_native import discover as native_discover


def work(fn):
    started = time.perf_counter_ns()
    try:
        value = fn()
        return value, None, time.perf_counter_ns()-started
    except (Unsupported, subprocess.CalledProcessError, ValueError, OSError) as exc:
        return None, str(exc)[:500], time.perf_counter_ns()-started


def graph_view(repo, entry, rev, roots, exe, db, name):
    if not roots:
        raise Unsupported('Excession indexed graph is not available for this language/scope')
    subprocess.run([str(exe),'index','--repo',str(repo),'--db',str(db),
                    '--name',name,'--rev',rev,'--roots',roots],
                   check=True, stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    root = roots.split(',')[0].rstrip('/')
    if not entry.startswith(root+'/'):
        raise Unsupported('entry must be under selected graph root')
    suffix = entry.replace('/','.').rsplit('.',1)[0]
    module = suffix
    result = subprocess.check_output([str(exe),'view','deps','--db',str(db),
                       '--name',name,'--rev',rev,'--seed',module])
    return {item['source_path'] for item in json.loads(result)['entries']}


def benchmark(repo, entry, language, rev='HEAD', roots=None, exe=None):
    with tempfile.TemporaryDirectory(prefix='excession-resolver-bench-') as temp:
        src, srcerr, srctime = work(lambda: static_discover(repo,rev,entry,language))
        nat, naterr, nattime = work(lambda: native_discover(repo,rev,entry,language))
        graph = grapherr = None
        graphtime = 0
        if roots and exe:
            graph, grapherr, graphtime = work(lambda: graph_view(repo,entry,rev,roots,exe,
                                    Path(temp)/'index.sqlite','benchmark'))
        static_paths = {x['path'] for x in src['inputs']} if src else None
        native_paths = {x['path'] for x in nat[0]['inputs']} if nat else None
        result = {'revision': subprocess.check_output(['git','-C',str(repo),'rev-parse',
                     '--verify',rev+'^{commit}'], text=True).strip(),
                  'language':language,'entry':entry,
                  'static':{'status':'OK' if src else 'UNSUPPORTED','elapsed_ns':srctime,
                            'paths':len(static_paths) if src else None,'reason':srcerr},
                  'native':{'status':'OK' if nat else 'UNSUPPORTED','elapsed_ns':nattime,
                            'paths':len(native_paths) if nat else None,
                            'bytes':nat[1]['declared_bytes'] if nat else None,
                            'staged_paths':nat[1]['staged_files'] if nat else None,
                            'reason':naterr},
                  'graph':{'status':'OK' if graph is not None else 'UNSUPPORTED',
                           'elapsed_ns':graphtime,'paths':len(graph) if graph is not None else None,
                           'reason':grapherr or ('graph not requested' if not roots else None)},
                  'comparisons':{}}
        if static_paths is not None and native_paths is not None:
            result['comparisons']['static_minus_native'] = sorted(static_paths-native_paths)
            result['comparisons']['native_minus_static'] = sorted(native_paths-static_paths)
        if graph is not None and native_paths is not None:
            result['comparisons']['graph_minus_native'] = sorted(graph-native_paths)
            result['comparisons']['native_minus_graph'] = sorted(native_paths-graph)
        return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('repo',type=Path)
    p.add_argument('entry')
    p.add_argument('--language',required=True,choices=['go','javascript','typescript','rust'])
    p.add_argument('--revision',default='HEAD')
    p.add_argument('--graph-roots',help='Excession index roots for JS/TS graph comparison')
    p.add_argument('--excession-bin',type=Path)
    p.add_argument('--trusted-local',action='store_true')
    args=p.parse_args()
    if not args.trusted_local:
        p.error('native compiler resolution requires --trusted-local')
    data=benchmark(args.repo,args.entry,args.language,args.revision,args.graph_roots,args.excession_bin)
    print(json.dumps(data,indent=2,sort_keys=True))

if __name__=='__main__':main()
