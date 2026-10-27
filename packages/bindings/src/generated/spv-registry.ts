export const abi = [
  {
    type: "constructor",
    inputs: [
      {
        name: "config",
        type: "tuple",
        internalType: "struct SpvRegistry.Configuration",
        components: [
          {
            name: "verifier",
            type: "address",
            internalType: "contract IRiscZeroVerifier",
          },
          {
            name: "image",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "policy",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "confirmations",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "minimumHeight",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "minimumWork",
            type: "uint256",
            internalType: "uint256",
          },
          {
            name: "observationPeriod",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "maximumAge",
            type: "uint32",
            internalType: "uint32",
          },
          {
            name: "futureDrift",
            type: "uint32",
            internalType: "uint32",
          },
        ],
      },
    ],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "accepted",
    inputs: [
      {
        name: "checkpoint",
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
    name: "confirmations",
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
    name: "finalize",
    inputs: [],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "fresh",
    inputs: [],
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
    name: "futureDrift",
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
    name: "halted",
    inputs: [],
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
    name: "headTimestamp",
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
    name: "maximumAge",
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
    name: "minimumHeight",
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
    name: "observationPeriod",
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
    name: "pending",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "tuple",
        internalType: "struct SpvJournal.Claim",
        components: [
          {
            name: "kind",
            type: "uint8",
            internalType: "uint8",
          },
          {
            name: "policy",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "previous",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "stable",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "head",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "maximumTime",
            type: "uint32",
            internalType: "uint32",
          },
        ],
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
    name: "proposedAt",
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
    name: "reportConflict",
    inputs: [
      {
        name: "seal",
        type: "bytes",
        internalType: "bytes",
      },
      {
        name: "claim",
        type: "tuple",
        internalType: "struct SpvJournal.Claim",
        components: [
          {
            name: "kind",
            type: "uint8",
            internalType: "uint8",
          },
          {
            name: "policy",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "previous",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "stable",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "head",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "maximumTime",
            type: "uint32",
            internalType: "uint32",
          },
        ],
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "stableHash",
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
    name: "stableHeight",
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
    name: "stablePrefix",
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
    name: "submit",
    inputs: [
      {
        name: "seal",
        type: "bytes",
        internalType: "bytes",
      },
      {
        name: "claim",
        type: "tuple",
        internalType: "struct SpvJournal.Claim",
        components: [
          {
            name: "kind",
            type: "uint8",
            internalType: "uint8",
          },
          {
            name: "policy",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "previous",
            type: "bytes32",
            internalType: "bytes32",
          },
          {
            name: "stable",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "head",
            type: "tuple",
            internalType: "struct SpvJournal.Point",
            components: [
              {
                name: "prefix",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "height",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "hash",
                type: "bytes32",
                internalType: "bytes32",
              },
              {
                name: "timestamp",
                type: "uint32",
                internalType: "uint32",
              },
              {
                name: "work",
                type: "uint256",
                internalType: "uint256",
              },
            ],
          },
          {
            name: "maximumTime",
            type: "uint32",
            internalType: "uint32",
          },
        ],
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
    name: "Advanced",
    inputs: [
      {
        name: "stable",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "height",
        type: "uint32",
        indexed: false,
        internalType: "uint32",
      },
      {
        name: "headWork",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "Conflict",
    inputs: [
      {
        name: "stable",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "alternative",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "headWork",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "Proposed",
    inputs: [
      {
        name: "head",
        type: "bytes32",
        indexed: true,
        internalType: "bytes32",
      },
      {
        name: "stable",
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
      {
        name: "observedAt",
        type: "uint64",
        indexed: false,
        internalType: "uint64",
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
  {
    type: "error",
    name: "ObservationPending",
    inputs: [],
  },
  {
    type: "error",
    name: "SourceHalted",
    inputs: [],
  },
  {
    type: "error",
    name: "StaleSource",
    inputs: [],
  },
] as const;
