# Source workload inventory

`source-inventory` parses length-prefixed raw blocks and emits operation counts. It checks framing, transaction Merkle roots, consecutive heights and parent hashes. It does not verify full consensus, measure VM cycles or produce settlement evidence.

`scripts/source_inventory.py` reads an existing node over loopback RPC and stores rows, coverage groups, sample candidates and a resumable frontier in MongoDB. A run binds the network, height interval, target block hash and scanner binary hash. A changed branch or binary cannot silently resume an existing run. Committed rows are idempotent, and conflicting rows fail instead of being overwritten.

Build the scanner with `cargo build --locked --release -p zutti-source --bin source-inventory`. Install the pinned Python dependencies from `inventory-requirements.txt` into an isolated runtime. Reuse an existing owner-only storage profile with `ZUTTI_MONGODB_URI` and `ZUTTI_MONGODB_DATABASE`, or an owner-only JSON file containing `uri` and `database`. The runner uses only the `source_inventory_runs` and `source_inventory_blocks` collections.

On Linux x86_64 with Python 3.12, the runtime installer verifies the locked wheel checksums and reuses cached downloads:

```sh
python3 deploy/source/install-inventory-runtime.py \
  --directory /path/to/inventory-runtime \
  --requirements deploy/source/inventory-requirements.txt
```

```sh
python3 scripts/source_inventory.py \
  --network testnet --run testnet-history \
  --rpc http://127.0.0.1:18232 --rpc-cookie /path/to/.cookie \
  --scanner /path/to/source-inventory --mongodb-config /path/to/inventory.json \
  --through-height 4529800 --max-blocks 2000 --max-seconds 300 \
  --max-rate 100 --out /path/to/summary.json
```

Choose a target height available on the node. Use `--node-container` when the RPC listener is in a container network namespace. The runner resolves its current process ID without publishing another network listener. Use `--mongodb-container` for an existing Docker-only MongoDB hostname; its alias must identify that container before the runner resolves its internal address. Credentials stay in the existing configuration file.

Re-run the same command to continue from its committed frontier. Increase `--max-blocks` and `--max-seconds` only within the host's resource budget. Run with a cgroup memory limit, CPU quota and low scheduling priority on shared hosts. An interrupted batch can be repeated without incrementing totals twice.

The summary is capped at 1 MB. Full rows and complete group statistics remain in MongoDB; at most 128 groups are exported, with an explicit truncation flag. Scan ETA uses the current session's observed throughput and is not proof time. Total cycles remain unset until representative execution measurements are joined to the inventory.

Run `python3 -m unittest discover -s scripts -p test_source_inventory.py`. Set `ZUTTI_INVENTORY_MONGO_CONFIG` to an inventory storage profile to include MongoDB interruption, conflict and concurrency tests. Set `ZUTTI_INVENTORY_MONGO_CONTAINER` when that profile uses a Docker-only hostname. Integration tests create and remove only uniquely named test collections.
