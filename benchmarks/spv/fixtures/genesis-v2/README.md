# Historical genuine genesis fixture

`receipt.bin` is the previously generated genuine composite receipt for the fixed Testnet genesis. `journal.bin` is its exact version-2 header claim.

- Image: `5b1db5eb5db64f783127cb2083c232a06e34c8a238e1aacb0a6c165c6b26b8bd`.
- ELF SHA-256: `f0493b320a609131992ae78e6f35eadcc5388baa6a284dc14411b561e073cf13`.
- Policy: Testnet; one confirmation; three-block window; valid through height four.
- Authenticated height: genesis only.

This is historical regression evidence, not the current release image or a deployment policy. Tests genuinely verify the receipt with development mode disabled and reject it as evidence for a different source program. This fixture cannot certify a newer program or a higher prefix.
