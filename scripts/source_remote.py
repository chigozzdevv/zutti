import gzip
import io
import json
import shlex
import subprocess
READ_METHODS = {'getblockchaininfo', 'getblockhash', 'getblockheader', 'getblock', 'getrawtransaction', 'z_gettreestate'}

class RemoteSource:

    def __init__(self, config):
        self.config = config

    def batch(self, requests, broadcast=False):
        allowed = READ_METHODS | ({'sendrawtransaction'} if broadcast else set())
        if not requests or len(requests) > 4096 or any((item['method'] not in allowed for item in requests)):
            raise ValueError('Unsupported source operation')
        agent = 'import base64,gzip,http.client,json,sys\nfrom pathlib import Path\nconfig=json.loads(sys.argv[1]);requests=json.load(sys.stdin)\nconnection=http.client.HTTPConnection("127.0.0.1",18232,timeout=30)\nauthorization="Basic "+base64.b64encode(Path(config["cookie"]).read_bytes().strip()).decode()\nresult=[];total=0\nfor index,item in enumerate(requests):\n connection.request("POST","/",json.dumps({"jsonrpc":"2.0","id":index,"method":item["method"],"params":item["params"]}),{"Content-Type":"application/json","Authorization":authorization})\n response=connection.getresponse();raw=response.read(4001025);total+=len(raw)\n if response.status!=200 or len(raw)>4001024 or total>24000000:raise RuntimeError("Source response exceeds its bound")\n value=json.loads(raw)\n if value.get("error") or value.get("result") is None:raise RuntimeError("Source RPC rejected "+item["method"])\n result.append(value["result"])\nsys.stdout.buffer.write(gzip.compress(json.dumps(result,separators=(",",":")).encode(),mtime=0))\n'
        outer = "import os,subprocess;pid=subprocess.check_output(['docker','inspect','--format','{{.State.Pid}}'," + repr(self.config['container']) + "],text=True).strip();os.execvp('nsenter',['nsenter','-t',pid,'-n','python3','-c'," + repr(agent) + ',' + repr(json.dumps({'cookie': self.config['cookie']})) + '])'
        result = subprocess.run(['ssh', '-S', self.config['control_socket'], '-oBatchMode=yes', '-oConnectTimeout=15', self.config['host'], 'python3 -c ' + shlex.quote(outer)], input=json.dumps(requests).encode(), capture_output=True, timeout=180, check=False)
        if result.returncode or len(result.stdout) > 24000000:
            detail = result.stderr.decode(errors='replace')[-2000:]
            raise RuntimeError('Source connection failed; no automatic transaction resubmission: ' + detail)
        with gzip.GzipFile(fileobj=io.BytesIO(result.stdout)) as compressed:
            raw = compressed.read(24_000_001)
        if len(raw) > 24_000_000:
            raise ValueError("Source response exceeds its decompression bound")
        values = json.loads(raw)
        if not isinstance(values, list) or len(values) != len(requests):
            raise ValueError('Incomplete source response')
        return values

    def call(self, method, params, broadcast=False):
        return self.batch([{'method': method, 'params': params}], broadcast)[0]

    def headers(self, first, last):
        if not 0 < first <= last < 500000000 or last - first >= 1024:
            raise ValueError('Invalid payment header range')
        hashes = self.batch([{'method': 'getblockhash', 'params': [height]} for height in range(first, last + 1)])
        values = self.batch([{'method': 'getblockheader', 'params': [value, False]} for value in hashes])
        if self.call('getblockhash', [last]) != hashes[-1]:
            raise ValueError('Source branch changed')
        return b''.join((bytes.fromhex(value) for value in values))
