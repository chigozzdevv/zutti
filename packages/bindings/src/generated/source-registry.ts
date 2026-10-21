export const abi = [
  {
    type: "constructor",
    inputs: [
      {
        name: "verifier_",
        type: "address",
        internalType: "contract IRiscZeroVerifier",
      },
      {
        name: "imageId_",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "genesis_",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "policy_",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "minimumWork_",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "accepted",
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
    name: "cumulativeWork",
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
    name: "genesis",
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
    name: "headState",
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
    name: "minimumWork",
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
    name: "policy",
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
    name: "stableCheckpoint",
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
    name: "update",
    inputs: [
      {
        name: "seal",
        type: "bytes",
        internalType: "bytes",
      },
      {
        name: "previous",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "state",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "checkpoint",
        type: "bytes32",
        internalType: "bytes32",
      },
      {
        name: "work",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
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
    name: "SourceUpdated",
    inputs: [
      {
        name: "state",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "checkpoint",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "work",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "error",
    name: "InvalidConfiguration",
    inputs: [],
  },
  {
    type: "error",
    name: "InvalidTransition",
    inputs: [],
  },
] as const;
