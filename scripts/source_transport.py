#!/usr/bin/env python3
import json
import os
from pathlib import Path
import re
import sys
from source_remote import RemoteSource
path = Path(os.environ['ZUTTI_OWNER_CONFIG'])
if path.is_symlink() or path.stat().st_mode & 0o077:
    raise ValueError('Private owner configuration required')
config = json.loads(path.read_text())
raw = sys.stdin.read(4000001)
if not re.fullmatch('(?:[0-9a-f]{2}){1,2000000}', raw):
    raise ValueError('Bounded transaction hex required')
source = RemoteSource(config['source'])
if source.call('getblockchaininfo', [])['chain'] != 'test':
    raise ValueError('Zcash Testnet node required')
txid = source.call('sendrawtransaction', [raw], broadcast=True)
if not isinstance(txid, str) or not re.fullmatch('[0-9a-f]{64}', txid):
    raise ValueError('Unexpected broadcast response')
print(txid)
