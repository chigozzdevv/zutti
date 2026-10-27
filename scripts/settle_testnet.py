import argparse
from concurrent.futures import ThreadPoolExecutor
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from ethereum_evidence import Transport, atomic_save, collect, request_digest
from source_remote import RemoteSource
from submit_testnet import Rpc
REPOSITORY = Path(__file__).resolve().parent.parent

class Settlement:

    def __init__(self, config, operation):
        if config.is_symlink() or config.stat().st_mode & 0o077:
            raise ValueError('Private owner configuration required')
        self.config_path = config
        self.config = json.loads(config.read_text())
        self.root = Path(self.config['root'])
        self.operation = operation.resolve()
        if self.operation.parent != self.root:
            raise ValueError('Operation must belong to this wallet runtime')
        self.deployment = json.loads(Path(self.config['deployment']).read_text())
        self.source = RemoteSource(self.config['source'])
        self.rpc = Rpc('https://ethereum-sepolia-rpc.publicnode.com')
        self.sequence = max((int(path.stem.split('-')[1]) for path in self.operation.glob('status-*.json')), default=0)

    def emit(self, stage, message, swap=None):
        self.sequence += 1
        value = {'operation_id': self.offer['intent_id'], 'swap_id': swap, 'stage': stage, 'message': message, 'blockers': [], 'next_action': 'none' if stage in {'settled', 'confirmed'} else 'wait', 'sequence': str(self.sequence)}
        atomic_save(self.operation / f'status-{self.sequence:04}.json', value)
        try:
            print(json.dumps({'type': 'status', 'value': value}), flush=True)
        except BrokenPipeError:
            pass

    def run(self, name, arguments, timeout=3600, env=None):
        output = self.operation / f'{name}.stdout'
        error = self.operation / f'{name}.stderr'
        with os.fdopen(os.open(output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'wb') as stdout, os.fdopen(os.open(error, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'wb') as stderr:
            result = subprocess.run(list(map(str, arguments)), cwd=REPOSITORY, stdout=stdout, stderr=stderr, timeout=timeout, env=env, check=False)
        if result.returncode:
            raise RuntimeError(f'{name} failed; inspect its private operation log')
        raw = output.read_text()
        try:
            return json.loads(raw.strip().splitlines()[-1])
        except (ValueError, IndexError):
            return None

    def native(self, name, role, command, *args):
        return self.run(name, [self.config['ethereum'], 'swap', '--deployment', self.config['deployment'], '--config', self.config['consensus'], '--directory', self.config[role], command, *args])

    def proof_start(self, name, kind, witness, dependencies, cycles):
        job = self.operation / f'{name}-job'
        binary = self.config['spv']
        if not job.exists():
            arguments = [binary, 'boundless', 'export', '--kind', kind, '--public', '--program', self.root / f"programs/{('source' if kind == 'source' else kind)}/program.elf", '--witness', witness, '--cycles', cycles, '--out', job]
            for dependency in dependencies:
                arguments += ['--dependency', dependency]
            self.run(name + '-export', arguments)
        self.run(name + '-execution', ['python3', REPOSITORY / 'crates/prover/spv/run.py', '--seconds', 900, '--rss-mib', 4096, '--log', self.operation / f'{name}-execution.log', '--', binary, 'boundless', 'execute', '--job', job / 'job.json'])
        request = Path(self.config['ledger']).parent / f'{name}-request.json'
        if not request.exists():
            self.run(name + '-submit', ['python3', REPOSITORY / 'scripts/prove_testnet.py', '--ledger', self.config['ledger'], '--config', self.config['prover_config'], '--binary', self.config['boundless'], '--job', job / 'job.json', '--name', name])
        return (job, request)

    def proof_finish(self, name, prepared):
        job, request = prepared
        fulfillment = self.operation / f'{name}-fulfillment.json'
        if not fulfillment.exists():
            self.run(name + '-collect', [self.config['boundless'], '--network', 'base', '--config', self.config['prover_config'], 'collect', '--request', request, '--out', fulfillment], timeout=7200)
        verified = self.operation / f'{name}-verified'
        if not verified.exists():
            self.run(name + '-import', [self.config['spv'], 'boundless', 'import', '--job', job / 'job.json', '--proof', fulfillment, '--out', verified])
        self.run(name + '-verify', [self.config['spv'], 'verify', '--program', job / 'program.elf', '--receipt', verified / 'receipt.bin', '--journal', job / 'journal.bin'])
        return verified / 'receipt.bin'

    def proof(self, name, kind, witness, dependencies, cycles):
        return self.proof_finish(name, self.proof_start(name, kind, witness, dependencies, cycles))

    def send(self, name, transaction):
        self.run(name, ['python3', REPOSITORY / 'scripts/submit_testnet.py', '--transaction', transaction, '--name', name, '--keystore', self.config['keystore'], '--password-file', self.config['password_file'], '--out', self.root / 'transactions'])
        return json.loads((self.root / f'transactions/{name}.json').read_text())

    def pool_send(self, name, witness, proof, role):
        operation = self.operation / f'{name}-operation.json'
        call = self.operation / f'{name}-call.json'
        if not operation.exists():
            self.native(name + '-export', role, 'pool-export', '--witness', witness, '--proof', proof, '--out', operation)
        if not call.exists():
            self.run(name + '-call', ['python3', REPOSITORY / 'scripts/pool_call.py', '--operation', operation, '--out', call])
        return self.send(name, call)

    def evidence(self, name, kind, request):
        captured = self.operation / f'{name}-capture.json'
        evidence = self.operation / f'{name}-evidence.json'
        arguments = ['python3', REPOSITORY / 'scripts/ethereum_evidence.py', '--kind', kind, '--config', self.config['consensus'], '--request', request, '--native', self.config['ethereum'], '--beacon', 'http://unstable.sepolia.beacon-api.nimbus.team', '--allow-http-beacon']
        if not captured.exists():
            self.run(name + '-capture', [*arguments, '--capture-only', '--out', captured])
        height = int(json.loads(captured.read_text())['execution']['number'], 16)
        deadline = time.monotonic() + 1800
        while not evidence.exists():
            transport = Transport('http://unstable.sepolia.beacon-api.nimbus.team', 'https://ethereum-sepolia-rpc.publicnode.com', True)
            finality = transport.rest('/eth/v1/beacon/light_client/finality_update')['finalized_header']
            block_hash = finality.get('execution_block_hash') or finality['execution']['block_hash']
            finalized = int(self.rpc.call('eth_getBlockByHash', [block_hash, False])['number'], 16)
            if finalized >= height:
                self.run(name + '-finality', [*arguments, '--capture', captured, '--out', evidence])
                break
            if time.monotonic() > deadline:
                raise RuntimeError('Ethereum finality is still pending')
            time.sleep(30)
        self.run(name + '-reverify', [self.config['ethereum'], kind, '--config', self.config['consensus'], '--request', request, '--evidence', evidence])
        return evidence

    def inventory(self, name, role, transaction):
        events = self.operation / f'{name}-events.json'
        inventory = self.operation / name
        if not events.exists():
            self.run(name + '-events', ['python3', REPOSITORY / 'scripts/pool_inventory.py', '--deployment', self.config['deployment'], '--block-hash', transaction['receipt']['blockHash'], '--out', events])
        if not inventory.exists():
            self.run(name + '-scan', [self.config['wallet'], 'inventory', '--directory', self.config[role], '--deployment', self.config['deployment'], '--events', events, '--out', inventory])
        return inventory

    def current_headers(self, name):
        info = self.source.call('getblockchaininfo', [])
        if info['chain'] != 'test':
            raise ValueError('Testnet source required')
        first = self.deployment['source_height'] + 1
        data = b'' if first > info['blocks'] else self.source.headers(first, info['blocks'])
        path = self.operation / f'{name}-headers.bin'
        if path.exists():
            path.unlink()
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'wb') as output:
            output.write(data)
        return path

    def execute(self):
        self.offer = json.loads((self.operation / 'selected-offer.json').read_text())
        order = self.operation / 'order'
        source_receipt = self.root / 'source-verified/receipt.bin'
        terms = json.loads((order / 'order.json').read_text())
        if terms['offer'] != self.offer['id'] or terms['usdc'] != self.offer['output']['amount'] or terms['zec'] != self.offer['input']['amount']:
            raise ValueError('Prepared payment does not match the selected offer')
        capture = self.operation / 'maker-inventory-capture.json'
        if not capture.exists():
            self.run('maker-inventory-early-capture', ['python3', REPOSITORY / 'scripts/ethereum_evidence.py', '--kind', 'note', '--config', self.config['consensus'], '--request', self.root / 'maker-note-request.json', '--native', self.config['ethereum'], '--beacon', 'http://unstable.sepolia.beacon-api.nimbus.team', '--allow-http-beacon', '--capture-only', '--out', capture])
        self.emit('preparing', 'Proving the prepared shielded payment')
        proven = order / 'proven.pczt'
        if not proven.exists():
            self.run('native-payment-proof', [self.config['wallet'], 'prove-payment', '--pczt', order / 'unsigned.pczt', '--terms', order / 'terms.bin', '--randomness', order / 'randomness.key', '--out', proven], timeout=1200)
        binding = self.proof('demo-binding-1', 'binding', order / 'binding-witness.bin', [], 1000000000)
        reservation = self.operation / 'reservation.bin'
        if not reservation.exists():
            self.run('share-reservation', [self.config['wallet'], 'share-order', '--directory', self.config['taker'], '--deployment', self.config['deployment'], '--order', order / 'order.backup', '--out', reservation])
        plan = self.operation / 'funding-plan'
        if not plan.exists():
            self.native('funding-plan', 'maker', 'funding-prepare', '--inventory', self.root / 'maker-inventory/inventory.backup', '--reservation', reservation, '--out', plan)
        self.emit('awaiting_reservation', 'Checking finalized maker inventory')
        funding = self.operation / 'funding'
        if not funding.exists():
            note_evidence = self.evidence('maker-inventory', 'note', plan / 'note-request.json')
            self.native('authorize-funding', 'maker', 'funding-authorize-note', '--plan', plan / 'plan.backup', '--binding', binding, '--anchor', source_receipt, '--evidence', note_evidence, '--headers', self.current_headers('funding'), '--out', funding)
        funding_proof = self.proof('demo-reservation-1', 'pool', funding / 'witness.bin', [binding, source_receipt], 1500000000)
        if not (self.root / 'transactions/maker-reservation.json').exists():
            self.native('check-funding-window', 'maker', 'funding-check', '--witness', funding / 'witness.bin',
                '--funding', funding_proof, '--source', source_receipt, '--headers', self.current_headers('reservation'))
        reservation_tx = self.pool_send('maker-reservation', funding / 'witness.bin', funding_proof, 'maker')
        payment = self.operation / 'payment'
        if not payment.exists():
            prepared = self.native('prepare-payment-journal', 'taker', 'payment-prepare', '--witness', funding / 'witness.bin', '--funding', funding_proof, '--pczt', proven, '--out', payment)
            atomic_save(self.operation / 'payment.json', prepared)
        swap = json.loads((self.operation / 'payment.json').read_text())['swap']
        sent_path = self.operation / 'source-payment.json'
        if sent_path.exists():
            sent = json.loads(sent_path.read_text())
        else:
            reservation_evidence = self.evidence('reservation', 'reservation', payment / 'request.json')
            self.emit('reserved', 'USDC reservation finalized and independently verified', swap)
            self.emit('awaiting_authorization', 'Checking current Zcash headers before signing', swap)
            env = {**os.environ, 'ZUTTI_OWNER_CONFIG': str(self.config_path)}
            sent = self.run('send-zcash', [self.config['ethereum'], 'swap', '--deployment', self.config['deployment'], '--config', self.config['consensus'], '--directory', self.config['taker'], 'payment-send', '--journal', payment, '--swap', swap, '--order', order / 'order.backup', '--evidence', reservation_evidence, '--source', source_receipt, '--continuation', self.current_headers('payment'), '--transport', REPOSITORY / 'scripts/source_transport.py'], env=env)
            atomic_save(sent_path, sent)
        self.emit('payment_confirming', 'Shielded Zcash payment broadcast', swap)
        deadline = time.monotonic() + 7200
        while True:
            tx = self.source.call('getrawtransaction', [sent['txid'], 1])
            if tx.get('blockhash'):
                header = self.source.call('getblockheader', [tx['blockhash'], True])
                height = header['height']
                if self.source.call('getblockchaininfo', [])['blocks'] >= height + 20:
                    break
            if time.monotonic() > deadline:
                raise RuntimeError('Zcash confirmations remain pending')
            time.sleep(30)
        all_headers = self.source.headers(self.deployment['source_height'] + 1, height + 20)
        headers = self.operation / 'payment-window-headers.bin'
        confirmations = self.operation / 'payment-window-confirmations.bin'
        block = self.operation / 'payment-block.bin'
        for path, data in [(headers, all_headers[:-14870]), (confirmations, all_headers[-14870:]), (block, bytes.fromhex(self.source.call('getblock', [tx['blockhash'], 0])))]:
            if not path.exists():
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(fd, 'wb') as output:
                    output.write(data)
        window = self.operation / 'window'
        admission = self.operation / 'admission'
        previous = self.config.get('admission_previous', {'receipt': str(source_receipt), 'height': self.deployment['source_height']})
        predecessor = Path(previous['receipt'])
        offset = previous['height'] - self.deployment['source_height']
        if offset < 0 or previous['height'] >= height + 10:
            raise ValueError('Invalid admitted source predecessor')
        admission_headers = self.operation / 'admission-headers.bin'
        if not admission_headers.exists():
            data = headers.read_bytes()[offset * 1487:]
            descriptor = os.open(admission_headers, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'wb') as output:
                output.write(data)
        if not window.exists():
            self.native('payment-window', 'taker', 'window-prepare', '--funding', funding / 'witness.bin', '--headers', headers, '--block', block, '--out', window)
        if not admission.exists():
            self.run('payment-admission', [self.config['spv'], 'prepare-admission', '--header-program', self.root / 'programs/source/program.elf', '--previous', predecessor, '--stable-headers', admission_headers, '--confirmations', confirmations, '--out', admission])
        self.emit('claimable', 'Proving confirmed payment and source admission', swap)
        window_job = self.proof_start('demo-window-1', 'source', window / 'witness.bin', [source_receipt], 4000000000)
        admission_job = self.proof_start('demo-admission-2', 'admission', admission / 'witness.bin', [predecessor], 4000000000)
        with ThreadPoolExecutor(max_workers=2) as executor:
            window_future = executor.submit(self.proof_finish, 'demo-window-1', window_job)
            admission_future = executor.submit(self.proof_finish, 'demo-admission-2', admission_job)
            window_proof = window_future.result()
            admission_proof = admission_future.result()
        admission_call = self.operation / 'source-advance-call.json'
        if not admission_call.exists():
            self.run('source-advance-export', [self.config['spv'], 'export-admission', '--program', self.root / 'programs/admission/program.elf', '--receipt', admission_proof, '--policy', self.deployment['source_policy_id'], '--registry', self.deployment['registry'].removeprefix('0x'), '--out', admission_call])
        advanced = self.send('source-payment-submit', admission_call)
        timestamp = int(self.rpc.call('eth_getBlockByHash', [advanced['receipt']['blockHash'], False])['timestamp'], 16)
        while int(self.rpc.call('eth_getBlockByNumber', ['latest', False])['timestamp'], 16) < timestamp + 120:
            time.sleep(15)
        self.send('source-payment-finalize', self.root / 'source-finalize-call.json')
        events = self.operation / 'reserved-pool-events.json'
        if not events.exists():
            self.run('reserved-pool-events', ['python3', REPOSITORY / 'scripts/pool_inventory.py', '--deployment', self.config['deployment'], '--block-hash', reservation_tx['receipt']['blockHash'], '--out', events])
        settlement = self.operation / 'settlement'
        if not settlement.exists():
            record = self.operation / 'recovery/swap.backup'
            if not record.exists():
                record = payment / 'swap.backup'
            self.native('prepare-settlement', 'taker', 'settlement-prepare', '--record', record, '--window', window / 'journal.bin', '--events', events, '--out', settlement)
        self.emit('settling', 'Proving the fixed private USDC payout', swap)
        payout_proof = self.proof('demo-payout-1', 'pool', settlement / 'witness.bin', [funding_proof, window_proof], 1500000000)
        payout = self.pool_send('swap-payout', settlement / 'witness.bin', payout_proof, 'taker')
        inventory = self.inventory('taker-inventory', 'taker', payout)
        owned = json.loads((inventory / 'inventory.json').read_text())['notes']
        if len(owned) != 1:
            raise ValueError('Expected one exact taker payout note')
        if not (self.operation / 'payout-note-request.json').exists():
            atomic_save(self.operation / 'payout-note-request.json', owned[0]['request'])
        evidence = self.evidence('payout', 'note', self.operation / 'payout-note-request.json')
        self.run('verify-owned-payout', [self.config['ethereum'], 'owned-note', '--config', self.config['consensus'], '--deployment', self.config['deployment'], '--directory', self.config['taker'], '--inventory', inventory / 'inventory.backup', '--evidence', evidence, '--position', owned[0]['position']])
        receipt = {'outcome': 'settled', 'swap_id': swap, 'received': self.offer['output'], 'source_payment': {'chain': 'zcash', 'block_hash': tx['blockhash'], 'height': str(height), 'transaction_id': sent['txid'], 'policy_id': self.deployment['source_policy_id']}, 'pool_settlement': {'chain': 'ethereum', 'block_hash': payout['receipt']['blockHash'], 'height': str(int(payout['receipt']['blockNumber'], 16)), 'transaction_id': payout['hash'], 'policy_id': self.deployment['source_policy_id']}, 'confirmed_at': int(time.time() * 1000)}
        if not (self.operation / 'settled-receipt.json').exists():
            atomic_save(self.operation / 'settled-receipt.json', receipt)
        else:
            receipt = json.loads((self.operation / 'settled-receipt.json').read_text())
        self.emit('settled', 'Private USDC payout finalized and ownership verified', swap)
        print(json.dumps({'type': 'receipt', 'value': receipt}), flush=True)

    def withdraw(self, request_path):
        self.offer = json.loads((self.operation / 'selected-offer.json').read_text())
        settled = json.loads((self.operation / 'settled-receipt.json').read_text())
        request = json.loads(request_path.read_text())
        if set(request) != {'asset', 'amount', 'to'} or request['asset'] != 'USDC':
            raise ValueError('An explicit USDC withdrawal is required')
        accepted = self.operation / 'withdrawal-request.json'
        if accepted.exists():
            if json.loads(accepted.read_text()) != request:
                raise ValueError('Withdrawal terms already fixed')
        else:
            atomic_save(accepted, request)
        inventory = self.operation / 'taker-inventory'
        notes = json.loads((inventory / 'inventory.json').read_text())['notes']
        if len(notes) != 1:
            raise ValueError('Exactly one owned payout note is required')
        swap = settled['swap_id']
        self.emit('preparing', 'Verifying the owned USDC note before withdrawal', swap)
        note_request = self.operation / 'withdrawal-note-request.json'
        if not note_request.exists():
            atomic_save(note_request, notes[0]['request'])
        try:
            evidence = self.evidence('payout', 'note', self.operation / 'payout-note-request.json')
        except RuntimeError:
            evidence = self.evidence('withdrawal-note', 'note', note_request)
        withdrawal = self.operation / 'withdrawal'
        if not withdrawal.exists():
            self.native('prepare-withdrawal', 'taker', 'withdraw', '--inventory', inventory / 'inventory.backup', '--position', notes[0]['position'], '--amount', request['amount'], '--owner', request['to'], '--evidence', evidence, '--out', withdrawal)
        proof = self.proof('demo-withdrawal-1', 'pool', withdrawal / 'witness.bin', [], 1000000000)
        self.emit('submitted', 'Submitting the proven public USDC withdrawal', swap)
        transaction = self.pool_send('taker-withdrawal', withdrawal / 'witness.bin', proof, 'taker')
        transfer_request = self.operation / 'withdrawal-transfer-request.json'
        if not transfer_request.exists():
            operation = json.loads((self.operation / 'taker-withdrawal-operation.json').read_text())['operation']
            atomic_save(transfer_request, {'chain_id': 11155111, 'token': self.deployment['token'], 'from': self.deployment['pool'], 'to': request['to'], 'value': hex(int(operation['value'])), 'transaction_id': transaction['hash']})
        transfer_evidence = self.operation / 'withdrawal-transfer-evidence.json'
        if not transfer_evidence.exists():
            block_hash = transaction['receipt']['blockHash']
            body_path = self.operation / 'withdrawal-block.json'
            if not body_path.exists():
                block = self.rpc.call('eth_getBlockByHash', [block_hash, True])
                receipts = self.rpc.call('eth_getBlockReceipts', [block_hash])
                atomic_save(body_path, {'transactions': block['transactions'], 'receipts': receipts})
            captured = self.operation / 'withdrawal-finality-capture.json'
            if not captured.exists():
                atomic_save(captured, {'schema': 1, 'request_digest': request_digest({}), 'execution': self.rpc.call('eth_getBlockByHash', [block_hash, False]), 'proofs': {}, 'finality_verified': False})
            capture = json.loads(captured.read_text())
            height = int(capture['execution']['number'], 16)
            deadline = time.monotonic() + 1800
            self.emit('confirming', 'Authenticating the USDC transfer against Ethereum finality', swap)
            while True:
                transport = Transport('http://unstable.sepolia.beacon-api.nimbus.team', 'https://ethereum-sepolia-rpc.publicnode.com', True)
                header = transport.rest('/eth/v1/beacon/light_client/finality_update')['finalized_header']
                final_hash = header.get('execution_block_hash') or header['execution']['block_hash']
                if int(self.rpc.call('eth_getBlockByHash', [final_hash, False])['number'], 16) >= height:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError('Withdrawal finality remains pending')
                time.sleep(30)
            consensus = json.loads(Path(self.config['consensus']).read_text())
            finality = collect(transport, consensus, {}, capture)['consensus']
            atomic_save(transfer_evidence, {'consensus': finality, **json.loads(body_path.read_text())})
        self.run('verify-withdrawal-transfer', [self.config['ethereum'], 'transfer', '--config', self.config['consensus'], '--request', transfer_request, '--evidence', transfer_evidence])
        receipt = {'operation_id': swap, 'kind': 'withdrawal', 'amount': {'asset': 'USDC', 'amount': request['amount']}, 'output_mode': 'public_erc20', 'to': request['to'], 'evidence': {'chain': 'ethereum', 'block_hash': transaction['receipt']['blockHash'], 'height': str(int(transaction['receipt']['blockNumber'], 16)), 'transaction_id': transaction['hash'], 'policy_id': self.deployment['source_policy_id']}, 'confirmed_at': int(time.time() * 1000)}
        output = self.operation / 'withdrawal-receipt.json'
        if not output.exists():
            atomic_save(output, receipt)
        else:
            receipt = json.loads(output.read_text())
        self.emit('confirmed', 'USDC withdrawal finalized and transfer independently verified', swap)
        print(json.dumps({'type': 'receipt', 'value': receipt}), flush=True)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--operation', type=Path, required=True)
    parser.add_argument('--withdraw', type=Path)
    args = parser.parse_args()
    runtime = Settlement(args.config, args.operation)
    with (runtime.operation / 'settlement.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.withdraw:
            runtime.withdraw(args.withdraw)
        else:
            runtime.execute()
if __name__ == '__main__':
    main()
