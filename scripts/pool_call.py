import argparse
import json
import re
import subprocess
from pathlib import Path
from ethereum_evidence import atomic_save
SIGNATURE = 'execute(bytes,(uint8,bytes32,bytes32,uint32,bytes32,bytes32,bytes32,uint64,address,bytes32[],(bytes32,bytes)[]))'

def encode(value):
    if value.get('chain_id') != 11155111 or value.get('proof_verified') is not True:
        raise ValueError('A verified Sepolia pool operation is required')
    if not re.fullmatch('0x[0-9a-fA-F]{40}', value['pool']):
        raise ValueError('Invalid pool address')
    if not re.fullmatch('0x[0-9a-fA-F]{520}', value['seal']):
        raise ValueError('A Groth16 Ethereum seal is required')
    operation = value['operation']
    nullifiers = '[' + ','.join(operation['nullifiers']) + ']'
    outputs = '[' + ','.join((f"({output['commitment']},{output['ciphertext']})" for output in operation['outputs'])) + ']'
    fields = [operation[key] for key in ('kind', 'root', 'checkpoint', 'expiry', 'orderTag', 'order', 'reservation', 'value', 'owner')]
    fields += [nullifiers, outputs]
    encoded = subprocess.run(['cast', 'calldata', SIGNATURE, value['seal'], '(' + ','.join(map(str, fields)) + ')'], capture_output=True, text=True, check=True, timeout=15).stdout.strip()
    if not re.fullmatch('0x(?:[0-9a-f]{2})+', encoded):
        raise ValueError('Invalid encoded pool call')
    return {'chain_id': 11155111, 'to': value['pool'], 'value': '0x0', 'data': encoded}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--operation', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if args.operation.stat().st_size > 150000:
        raise ValueError('Operation exceeds its bound')
    value = encode(json.loads(args.operation.read_text()))
    atomic_save(args.out, value)
    print(json.dumps({'pool_call_prepared': True, 'transaction_submitted': False}))
if __name__ == '__main__':
    main()
