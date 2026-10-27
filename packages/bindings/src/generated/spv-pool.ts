export const abi = [
  {
    type: "constructor",
    inputs: [
      {
        name: "config",
        type: "tuple",
        internalType: "struct SpvPool.Configuration",
        components: [
          {
            name: "token",
            type: "address",
            internalType: "contract ISpvToken",
          },
          {
            name: "verifier",
            type: "address",
            internalType: "contract IRiscZeroVerifier",
          },
          {
            name: "source",
            type: "address",
            internalType: "contract SpvRegistry",
          },
          {
            name: "poolImage",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "bindingImage",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "minimumRemaining",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "proofProfile",
            type: "bytes32",
            internalType: "bytes32",
          },
        ],
      },
    ],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "PAYMENT_PROFILE",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "SOURCE_GENESIS",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "backing",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "bindingImage",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "commitments",
    inputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "domain",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "execute",
    inputs: [
      {
        name: "seal",
        type: "bytes",
        internalType: "bytes",
      },
      {
        name: "op",
        type: "tuple",
        internalType: "struct SpvPool.Operation",
        components: [
          {
            name: "kind",
            type: "uint8",
            internalType: "uint8",
          },
          {
            name: "root",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "checkpoint",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "expiry",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "orderTag",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "order",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "reservation",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "value",
            type: "uint64",
            internalType: "uint64",
          },
          {
            name: "owner",
            type: "address",
            internalType: "address",
          },
          {
            name: "nullifiers",
            type: "bytes32[]",
            internalType: "bytes32[]",
          },
          {
            name: "outputs",
            type: "tuple[]",
            internalType: "struct SpvPool.Output[]",
            components: [
              {
                name: "commitment",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "ciphertext",
                type: "bytes",
                internalType: "bytes",
              },
            ],
          },
        ],
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "imageId",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "journalDigest",
    inputs: [
      {
        name: "op",
        type: "tuple",
        internalType: "struct SpvPool.Operation",
        components: [
          {
            name: "kind",
            type: "uint8",
            internalType: "uint8",
          },
          {
            name: "root",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "checkpoint",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "expiry",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "orderTag",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "order",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "reservation",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "value",
            type: "uint64",
            internalType: "uint64",
          },
          {
            name: "owner",
            type: "address",
            internalType: "address",
          },
          {
            name: "nullifiers",
            type: "bytes32[]",
            internalType: "bytes32[]",
          },
          {
            name: "outputs",
            type: "tuple[]",
            internalType: "struct SpvPool.Output[]",
            components: [
              {
                name: "commitment",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "ciphertext",
                type: "bytes",
                internalType: "bytes",
              },
            ],
          },
        ],
      },
    ],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "minimumRemaining",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "uint32",
        internalType: "uint32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "nextLeaf",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "uint64",
        internalType: "uint64",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "orders",
    inputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "root",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "roots",
    inputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "source",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "contract SpvRegistry",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "spent",
    inputs: [
      {
        name: "",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "token",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "contract ISpvToken",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "verifier",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "contract IRiscZeroVerifier",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "event",
    name: "NullifierSpent",
    inputs: [
      {
        name: "nullifier",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "OutputInserted",
    inputs: [
      {
        name: "position",
        type: "uint32",
        indexed: true,
        internalType: "uint32",
      },
      {
        name: "commitment",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "ciphertext",
        type: "bytes",
        indexed: false,
        internalType: "bytes",
      },
    ],
    anonymous: false,
  },
  {
    type: "error",
    name: "AlreadySpent",
    inputs: [],
  },
  {
    type: "error",
    name: "DuplicateCommitment",
    inputs: [],
  },
  {
    type: "error",
    name: "InvalidConfiguration",
    inputs: [],
  },
  {
    type: "error",
    name: "InvalidOperation",
    inputs: [],
  },
  {
    type: "error",
    name: "Reentrancy",
    inputs: [],
  },
  {
    type: "error",
    name: "TokenTransfer",
    inputs: [],
  },
  {
    type: "error",
    name: "TreeFull",
    inputs: [],
  },
  {
    type: "error",
    name: "UnknownCheckpoint",
    inputs: [],
  },
  {
    type: "error",
    name: "UnknownRoot",
    inputs: [],
  },
] as const;
