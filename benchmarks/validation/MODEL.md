# Testnet source cycle model

The first planning scenarios project **1.97–2.41 quadrillion RISC Zero user cycles** for source execution over the pinned Testnet snapshot, through height **4,529,880**. This extrapolates measured samples; the complete history has not been executed in the VM. The two scenarios are not guaranteed bounds or a proving quote.

The strongest component projection is **1.809 quadrillion cycles for Orchard and Ironwood authorization**, assuming shared setup within fixed groups of 128 blocks. Other source work uses less complete measurements and explicit proxies.

## Completed authorization measurements

All three suites used the same diagnostic program, checked genuine public transactions natively and in the VM, and matched their input-bound journals. No receipts were generated.

| Circuit | First verification, including setup | Later smaller bundle | Later larger bundle |
| --- | ---: | ---: | ---: |
| Original Orchard | 131.924 billion | 1.781 billion, 2 actions | 2.803 billion, 6 actions |
| Fixed Orchard | 131.670 billion | 1.526 billion, 1 action | 1.775 billion, 2 actions |
| Ironwood | 131.662 billion | 1.518 billion, 1 action | 1.776 billion, 2 actions |

The difference between the first and repeated smaller verification is approximately **130.14 billion cycles** for each circuit. This is shared first-use work, including verifier-key construction and arithmetic initialization. It is separate from parsing in these suites.

The fixed batch partition contains 4,949 original-Orchard, 1,006 fixed-Orchard and 2,871 Ironwood setup occurrences. Applying the fitted setup cost produces **1.149 quadrillion cycles**, about **63.5%** of the modern-authorization projection. The remaining **660.2 trillion cycles** are fitted bundle/action verification work.

## Planning scenarios

Values below are **trillion user cycles**. The first scenario transfers an early 16/128-block source fit across history. The higher-overhead scenario charges the measured 91.9-million-cycle single-block common-work control for every block and uses broader transaction/context allowances.

| Work | Batch model | Higher-overhead model |
| --- | ---: | ---: |
| Orchard and Ironwood authorization | 1,808.8 | 1,808.8 |
| Common block processing proxy | 66.9 | 416.3 |
| Legacy Sprout authorization proxy | 11.8 | 13.1 |
| Sapling authorization proxy | 20.2 | 26.7 |
| Shielded tree update projection | 35.0 | 35.0 |
| Shielded coinbase context proxy | 23.7 | 52.5 |
| Additional ledger/script proxy | 4.7 | 60.7 |
| **Total** | **1,971.1** | **2,413.2** |

The ledger/script scenarios use a historical transparent-transaction delta, applied per additional transaction or per additional transparent state operation. Actual executed multisig counts and representative later-state witness costs remain unmeasured. Legacy rates also come from different recorded program versions. These transfers and allowances are modeling assumptions.

Coinbase counts are exact for single-transaction blocks. Those identify 256,165 Sapling outputs, 121,903 Orchard actions and 71,501 Ironwood actions requiring coinbase recovery. Additional shielded coinbases occur in other blocks. The scenarios charge identified recovery work versus all shielded outputs/actions and include shared recovery setup.

Tree measurements start from synthetic frontiers and repeatedly append real public commitments. They establish component work, not the exact cost of reconstructing and updating every historical frontier.

## Scope and limits

- The target is the pinned public Testnet snapshot. Newer blocks are additional work.
- Fixed groups of 128 blocks have not been shown to satisfy every witness and execution bound. More splits repeat setup and increase the estimate.
- The current 128-block maximum implies at least **35,390 source batches** after genesis. This is not a paid-request count or a demonstrated proving schedule.
- Authorization fits use 1/2-action or 2/6-action samples. Larger bundles are extrapolated.
- Paging, segment padding, recursive proving, compression, Ethereum wrapping, retries and private swap proofs are excluded.
- CPU execution wall time does not establish GPU proving latency, fees or bootstrap completion time.
- Diagnostic programs are not admitted source-verifier images. No current-chain source state or swap settlement was admitted by these runs.

The complete proving budget remains unestablished. **No paid proving jobs were submitted.**

## Reproduce

Raw circuit results are in `results/2026-10-23/`. Completed context/tree/control measurements are in `../source-validation-components-2026-10-23.json`. The complete inventory and refined distributions are bound by SHA-256 digests in `../source-cycle-model-2026-10-23.json`.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/validation/plan.py \
  --out /tmp/zutti-source-model.json

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s benchmarks/validation -p 'test_*.py'
```

The next optimization target is repeated deterministic public verifier setup. Precomputing and pinning equivalent verifier data is a candidate; its implementation, verification checks and cycle savings still need demonstration. The remaining verification work and other source costs would still have to be proved.
