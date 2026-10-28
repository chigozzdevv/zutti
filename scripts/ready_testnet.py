import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from ethereum_evidence import Transport, atomic_save, collect
from source_remote import RemoteSource
from prove_testnet import private_json


REPOSITORY = Path(__file__).resolve().parent.parent


def invoke(binary, *arguments):
    result = subprocess.run([str(binary), *map(str, arguments)], cwd=REPOSITORY,
                            capture_output=True, text=True, timeout=90, check=False)
    if result.returncode or len(result.stdout) > 65_536:
        raise RuntimeError('Readiness verification failed; no quote may be issued')
    return json.loads(result.stdout.strip().splitlines()[-1])


def program_inputs(config, deployment):
    result = {}
    for kind in ('source', 'admission', 'binding', 'pool'):
        directory = Path(config['root']) / 'programs' / kind
        manifest = json.loads((directory / 'program.json').read_text())
        digest = hashlib.sha256((directory / 'program.elf').read_bytes()).hexdigest()
        if (manifest['image_id'] != deployment[f'{kind}_image']
                or manifest['program_sha256'] != digest):
            raise ValueError('Proving programs do not match the deployed profile')
        result[kind] = manifest
    return result


def check_authorization(ledger, programs):
    scope = ledger.get('scope', {})
    if (ledger.get('network') != 'base' or ledger.get('automatic_resubmission') is not False
            or ledger.get('testnet_witnesses_public') is not True
            or scope.get('kind') != 'single_testnet_swap' or scope.get('completed') is not False
            or any(program['image_id'] not in scope.get('programs', []) for program in programs.values())):
        raise ValueError('An open proof approval for this exact swap profile is required before quoting')
    records = ledger['requests']
    maximum = ledger['maximum_requests']
    remaining = int(ledger['total_cap_wei']) - sum(int(record['reserved_wei']) for record in records)
    if (not 0 < int(ledger['per_request_cap_wei']) <= remaining
            or (maximum is not None and (type(maximum) is not int or len(records) >= maximum))):
        raise ValueError('Proof approval has no capacity for new work')


def preflight(config_path, output):
    if config_path.is_symlink() or config_path.stat().st_mode & 0o077:
        raise ValueError('Private owner configuration required')
    config = json.loads(config_path.read_text())
    deployment = json.loads(Path(config['deployment']).read_text())
    if deployment.get('network') != 'testnet' or deployment.get('chain_id') != 11155111:
        raise ValueError('Sepolia Testnet deployment required')
    programs = program_inputs(config, deployment)
    check_authorization(private_json(Path(config['ledger'])), programs)
    if output.exists():
        raise ValueError('Readiness output already exists')
    root = Path(config['root'])
    source = Path(config.get('source_receipt', root / 'source-verified/receipt.bin'))
    height = config.get('source_height', deployment['source_height'])
    inventory = Path(config.get('maker_inventory', root / 'maker-inventory/inventory.backup'))
    position = config.get('maker_position', 0)
    if type(position) is not int or position < 0:
        raise ValueError('Invalid maker note position')
    with tempfile.TemporaryDirectory(prefix='.ready-', dir=output.parent) as temporary:
        stage = Path(temporary)
        request = stage / 'request.json'
        native = [
            'swap', '--deployment', config['deployment'], '--config', config['consensus'],
            '--directory', config['maker'],
        ]
        owner = ['--inventory', inventory, '--position', position, '--source', source]
        invoke(config['ethereum'], *native, 'ready-request', *owner, '--out', request)
        keys = invoke(config['ethereum'], 'proof-keys', '--kind', 'ready', '--request', request)
        transport = Transport(config.get('beacon', 'http://unstable.sepolia.beacon-api.nimbus.team'),
                              config.get('ethereum_rpc', 'https://ethereum-sepolia-rpc.publicnode.com'),
                              config.get('allow_http_beacon', True))
        evidence = stage / 'evidence.json'
        retained = config.get('readiness_capture')
        captured = private_json(Path(retained)) if retained else None
        atomic_save(evidence, collect(transport, json.loads(Path(config['consensus']).read_text()), keys, captured))
        captured = json.loads(evidence.read_text())
        atomic_save(stage / 'note-evidence.json', {'consensus': captured['consensus'], 'pool': captured['pool']})
        atomic_save(stage / 'note-request.json', json.loads(request.read_text())['note'])
        remote = RemoteSource(config['source'])
        info = remote.call('getblockchaininfo', [])
        if (info.get('chain') != 'test' or type(info.get('blocks')) is not int
                or type(height) is not int or not 0 <= info['blocks'] - height <= 512):
            raise ValueError('Refresh shared source evidence before quoting')
        data = b'' if height == info['blocks'] else remote.headers(height + 1, info['blocks'])
        headers = stage / 'headers.bin'
        headers.write_bytes(data)
        os.chmod(headers, 0o600)
        verified = invoke(config['ethereum'], *native, 'ready', *owner, '--headers', headers, '--evidence', evidence)
        if (verified.get('ready') is not True or verified.get('domain') != deployment['domain']
                or verified.get('source_height') != height
                or verified.get('current_height') != info['blocks']
                or verified.get('source_policy_id') != deployment['source_policy_id']):
            raise ValueError('Readiness result does not match the selected deployment')
        report = {
            'version': 1, 'checked_at': int(time.time()), 'verified': verified,
            'admission_mode': 'payment-window' if 'payment-window' in programs['admission'].get('admission_inputs', []) else 'headers',
            'source_receipt_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'inventory_sha256': hashlib.sha256(inventory.read_bytes()).hexdigest(),
            'evidence_sha256': hashlib.sha256(evidence.read_bytes()).hexdigest(),
            'request_sha256': hashlib.sha256(request.read_bytes()).hexdigest(),
            'download_bytes': transport.total, 'paid_jobs': 0, 'funded': False,
        }
        atomic_save(stage / 'ready.json', report)
        os.rename(stage, output)
        descriptor = os.open(output.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    return report


def main():
    parser = argparse.ArgumentParser(description='Verify source admission and owned maker inventory before Testnet RFQ')
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(preflight(args.config, args.out)))


if __name__ == '__main__':
    main()
