# GPU source prover

This image runs one public Testnet source job with RISC Zero 3.0.5 and native CUDA Groth16 wrapping. It does not require Docker inside the Pod. The worker selects GPU 0 and processes one job; it does not distribute a job across several GPUs.

The container build and GPU execution still require validation on Linux with NVIDIA hardware. Local host and runner tests do not establish GPU throughput, memory requirements or bootstrap cost.

## Build

Export the exact guest programs from the current build. Commit source changes before preparing the context:

```sh
cargo build --locked -p zutti-prover --bin source-prover --bin private-prover
cargo run --locked -p zutti-prover --bin private-prover -- \
  export-programs --out /tmp/zutti-programs
python3 deploy/prover/prepare.py \
  --programs /tmp/zutti-programs --out /tmp/zutti-prover-context
docker buildx build --platform linux/amd64 --load \
  --tag zutti-prover:local /tmp/zutti-prover-context
```

Preparation copies committed Cargo sources and verification keys, plus guest ELFs whose SHA-256 hashes match the committed release manifest. The build checks their image IDs as well. It excludes the Git repository, local data, credentials and job inputs. No repository token is needed inside the image. `build.json` records the commit and program identities.

Use a private registry when making the image available to Runpod. Image publication and Pod rental are separate operator actions; these scripts do neither.

## Run

Mount a durable workspace at `/workspace`. Provision enough disk for execution traces and intermediate proofs. Start with at least 32 GiB RAM and 200 GiB disk; these are initial provisioning targets, not measured requirements for the busy block. CPU capacity also affects execution and proving. The default disk check requires 100 GiB free before starting a new proof.

Export a real source range using its complete transition witness:

```sh
source-prover export-range \
  --witness /workspace/range.bin --programs /workspace/programs \
  --cycle-limit 100000000000 --out /workspace/job
```

The 100-billion-cycle bound leaves room above the measured 82.8-billion-cycle block. It is an execution ceiling, not an estimate of proving time. Existing Boundless submissions retain their 2.1-billion-cycle ceiling and spending checks.

On a Linux GPU host, this is the container invocation:

```sh
docker run --rm --gpus 'device=0' \
  --mount type=bind,src=/absolute/workspace,dst=/workspace \
  zutti-prover:local \
  --job /workspace/job --output /workspace/results \
  --cycle-limit 100000000000 --segment-po2 20 --max-seconds 14400
```

For Runpod, use the same image and arguments in a custom Pod template with a persistent volume mounted at `/workspace`. Do not invoke `docker run` inside the Pod. The image entrypoint runs `run.py` directly. Without arguments, it prints help and exits.

This image exposes no SSH or notebook service. Preload the public job onto the network volume through Runpod's [S3-compatible volume API](https://docs.runpod.io/storage/s3-api), then download the results through that API. Keep storage credentials on the controlling host; the prover needs only mounted files. Confirm S3 access for the selected volume region before renting. Runpod documents [custom Pod templates](https://docs.runpod.io/pods/templates/create-custom-template) and [persistent network volumes](https://docs.runpod.io/storage/network-volumes).

Run a small real Testnet source job first to validate CUDA proving, Groth16 wrapping and result verification. Then run block 282,757 and representative later witnesses. Compare segment sizes 20, 21 and 22 only after obtaining a genuine proof at 20. Record complete wall time, peak resources and the actual rental rate; execution-only timing does not establish proving throughput.

The worker snapshots bounded regular input files, rejects symlinks and nonpublic manifests, checks the admitted source program and Testnet genesis, and requests a Groth16 receipt with development mode disabled. It verifies the receipt against the submitted job before recording success or exporting the 260-byte EVM seal.

Results are stored under `results/<job_id>/`: `source.proof`, `seal.bin`, `verified.json`, `timing.json`, `hardware.json` and `prove.log`. Keep these on durable storage and copy them back before terminating the Pod. A restart reuses a completed proof only after verifying it again. Interrupted proofs restart from that job; there is no segment-level checkpoint recovery.

`--max-seconds` stops the proving process group when the work window expires. It does not stop or delete the Pod, release its GPU, or cap the provider's bill. Stop billing through the provider after copying results, including when a job fails.

## Verify independently

On a separate trusted host with the same admitted program definitions:

```sh
source-prover verify \
  --job /absolute/original/job --proof /absolute/results/source.proof \
  --seal /absolute/results/independently-verified-seal.bin
```

A range receipt proves that range's transition. It still needs verified ancestry and aggregation before it can contribute to current-chain admission. A valid receipt alone does not establish fork selection, settlement readiness or an end-to-end swap.

Only public chain data belongs in this worker. Do not send wallet keys, viewing keys or real private swap witnesses. Source proving changes no committee or custody assumptions. Private proving remains a separate privacy boundary.

## Local checks

```sh
python3 -m unittest discover -s deploy/prover -p 'test_*.py' -v
cargo test --locked -p zutti-prover --test source --bin source-prover
```

Runner tests use mocked subprocess results to exercise routing, recovery and failure handling. Rust tests verify retained genuine receipts and reject tampering and fake receipts. Neither test suite substitutes for a genuine GPU proof from the image.
