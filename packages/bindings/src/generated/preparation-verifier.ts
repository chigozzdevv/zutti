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
        name: "domain_",
        type: "bytes32",
        internalType: "bytes32",
      },
    ],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "VERIFICATION_GAS",
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
    type: "function",
    name: "verify",
    inputs: [
      {
        name: "seal",
        type: "bytes",
        internalType: "bytes",
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
    ],
    outputs: [],
    stateMutability: "view",
  },
  {
    type: "error",
    name: "InvalidCommitment",
    inputs: [],
  },
  {
    type: "error",
    name: "InvalidConfiguration",
    inputs: [],
  },
] as const;
