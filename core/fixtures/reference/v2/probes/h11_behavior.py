"""Cross-layout observational probe over the real h11 protocol library."""
import hashlib
import json
from pathlib import Path
import h11
from h11 import CLIENT, SERVER, Connection, Request, Response, Data, EndOfMessage

assert Path(h11.__file__).resolve().is_relative_to(Path.cwd()), h11.__file__
out=[]
for i in range(100):
    client=Connection(CLIENT)
    server=Connection(SERVER)
    method=b'GET' if i%2==0 else b'POST'
    target=('/item/'+str(i)).encode()
    req=Request(method=method,target=target,headers=[(b'host',b'example.org')])
    wire=client.send(req)+client.send(EndOfMessage())
    server.receive_data(wire)
    received=server.next_event()
    assert isinstance(received,Request)
    assert received.target==target and received.method==method
    assert isinstance(server.next_event(),EndOfMessage)
    payload=('result-'+str(i)).encode()
    headers=[(b'content-length',str(len(payload)).encode())]
    resp=server.send(Response(status_code=200,headers=headers))
    resp+=server.send(Data(data=payload))
    resp+=server.send(EndOfMessage())
    client.receive_data(resp)
    assert isinstance(client.next_event(),Response)
    assert client.next_event().data==payload
    assert isinstance(client.next_event(),EndOfMessage)
    out.append([wire.hex(),resp.hex()])
print(json.dumps({'sha256':hashlib.sha256(json.dumps(out).encode()).hexdigest(),
                  'conversations':len(out), 'all':list(h11.__all__),'version':h11.__version__},sort_keys=True))
