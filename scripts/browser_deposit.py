import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def private(path, maximum):
    stat = path.lstat()
    if path.is_symlink() or not path.is_file() or stat.st_mode & 0o077 or stat.st_size > maximum:
        raise ValueError('Private bounded deposit artifacts required')
    return path.read_bytes()


def run(arguments, timeout=120):
    result = subprocess.run(list(map(str, arguments)), capture_output=True, timeout=timeout, check=True)
    return result.stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['prepare', 'collect'])
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--request', type=Path)
    args = parser.parse_args()
    config = json.loads(private(args.config, 150000))
    directory = args.directory
    if directory.is_symlink() or not directory.is_dir() or directory.stat().st_mode & 0o077:
        raise ValueError('Private deposit directory required')
    witness = private(directory / 'witness.bin', 16384)
    if not witness.startswith(b'ZUTTISPV\x02\x00'):
        raise ValueError('SPV pool witness required')
    job = directory / 'job'
    binary = config['spv']
    if args.action == 'prepare':
        if args.request:
            raise ValueError('Preparation cannot submit or collect a proof')
        if not job.exists():
            run([binary, 'boundless', 'export', '--kind', 'pool', '--public', '--program',
                 Path(config['root']) / 'programs/pool/program.elf', '--witness', directory / 'witness.bin',
                 '--cycles', '12000000', '--out', job])
        executed = run([binary, 'boundless', 'execute', '--job', job / 'job.json'], timeout=180)
        verified = json.loads(executed.strip().splitlines()[-1])
        if verified.get('journal_matches') is not True or verified.get('host_assumptions') != 0:
            raise ValueError('Deposit execution did not verify')
        print(json.dumps({'job': str(job / 'job.json'), 'execution': verified, 'proof_submitted': False}))
        return
    if not args.request:
        raise ValueError('Collection requires an existing explicitly authorized proof request')
    private(args.request, 150000)
    fulfillment = directory / 'fulfillment.json'
    if not fulfillment.exists():
        run([config['boundless'], '--network', 'base', '--config', config['prover_config'],
             'collect', '--request', args.request, '--out', fulfillment], timeout=180)
    verified = directory / 'verified'
    if not verified.exists():
        run([binary, 'boundless', 'import', '--job', job / 'job.json', '--proof', fulfillment, '--out', verified])
    receipt = private(verified / 'receipt.bin', 16 * 1024 * 1024)
    target = directory / 'receipt.bin'
    if target.exists():
        if private(target, 16 * 1024 * 1024) != receipt:
            raise ValueError('Retained deposit receipt differs')
    else:
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=directory, delete=False) as output:
                temporary = output.name
                os.chmod(temporary, 0o600)
                output.write(receipt)
                output.flush()
                os.fsync(output.fileno())
            os.link(temporary, target)
            descriptor = os.open(directory, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        finally:
            if temporary:
                os.unlink(temporary)
    print(json.dumps({'proof_verified': True, 'transaction_submitted': False}))


if __name__ == '__main__':
    main()
