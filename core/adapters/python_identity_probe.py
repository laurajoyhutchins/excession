"""Isolated Python reflection and pickle witness for the pinned h11 module move."""
from __future__ import annotations

import base64
import importlib
import inspect
import json
from pathlib import Path
import pickle
import sys

from python_identity import declared_symbols

root=Path.cwd().resolve()
old=importlib.import_module('h11._headers')
physical=importlib.import_module('h11.wire._headers') if (root/'h11/wire/_headers.py').exists() else old
module_path=Path(inspect.getsourcefile(old.Headers)).resolve()
raw=module_path.read_bytes()
symbols=declared_symbols(raw)


def object_for(name):
    value=old
    for token in name.split('.'):
        value=getattr(value,token)
    return value


rows={}
blobs={}
for name in symbols:
    obj=object_for(name)
    row={'module':obj.__module__,'qualname':obj.__qualname__,
         'signature':str(inspect.signature(obj))}
    for protocol in (4,5):
        data=pickle.dumps(obj,protocol=protocol)
        row[f'pickle_protocol_{protocol}']=data.hex()
        blobs[name+f'-p{protocol}']=base64.b64encode(data).decode('ascii')
    row['unpickle_same']=all(pickle.loads(pickle.dumps(obj,protocol=p)) is obj for p in (4,5))
    rows[name]=row

value=old.normalize_and_validate([('x-test','hi')])
instance={}
for protocol in (4,5):
    data=pickle.dumps(value,protocol=protocol)
    instance[f'pickle_protocol_{protocol}']=data.hex()
    blobs['Headers-instance'+f'-p{protocol}']=base64.b64encode(data).decode('ascii')
    recovered=pickle.loads(data)
    instance[f'roundtrip_protocol_{protocol}']=bool(type(recovered) is type(value) and list(recovered)==list(value))
instance['repr']=repr(value)

# Cross-process loading uses protocol 4, which is commonly stored in older
# application snapshots. Protocol 5 is covered by exact byte equality above.
cross={name:blobs[name+'-p4'] for name in symbols}
cross['Headers-instance']=blobs['Headers-instance-p4']

print(json.dumps({
    'objects':rows,
    'instance':instance,
    'pickle_bytes':cross,
    'module_name':old.__name__,
    'spec_name':old.__spec__.name,
    'source_path':module_path.relative_to(root).as_posix(),
    'legacy_alias_identical':old is physical,
    'class_identity':old.Headers is physical.Headers,
},sort_keys=True))
