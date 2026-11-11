"""Native Python proof of cross-layout pickle lookup against old public names."""
import base64,importlib,json,pickle,sys
cases=json.load(sys.stdin)
for name,payload in cases.items():
    obj=pickle.loads(base64.b64decode(payload))
    if name=='Headers-instance':
        from h11._headers import Headers
        assert type(obj) is Headers and list(obj)==[(b'x-test',b'hi')]
    else:
        current=importlib.import_module('h11._headers')
        for part in name.split('.'):
            current=getattr(current,part)
        assert obj is current
print('PICKLE_CROSS_LOAD_OK')
