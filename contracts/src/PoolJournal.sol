// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

library PoolJournal {
    struct Claim {
        uint8 kind;
        bytes32 domain;
        bytes32 root;
        bytes32 checkpoint;
        bytes32 orderTag;
        bytes32 order;
        bytes32 reservation;
        uint64 value;
        address owner;
        bytes32 nullifiers;
        bytes32 outputs;
    }

    function digest(Claim memory claim) internal pure returns (bytes32) {
        return sha256(
            abi.encodePacked(
                bytes8("ZUTTIPOL"),
                hex"0100",
                claim.kind,
                claim.domain,
                claim.root,
                claim.checkpoint,
                claim.orderTag,
                claim.order,
                claim.reservation,
                little64(claim.value),
                claim.owner,
                claim.nullifiers,
                claim.outputs
            )
        );
    }

    function deploymentDomain(
        bytes32 genesis,
        bytes32 policy,
        bytes32 paymentProfile,
        bytes32 proofProfile,
        uint64 chain,
        address pool,
        address token
    ) internal pure returns (bytes32) {
        bytes memory deployment = abi.encodePacked(
            bytes8("ZUTTIDOM"), hex"0100", genesis, little64(chain), pool, token, paymentProfile, policy, proofProfile
        );
        return sha256(
            abi.encodePacked(
                hex"5a555454490053484132353601",
                hex"0600000000000000",
                bytes6("domain"),
                hex"0100000000000000",
                hex"ba00000000000000",
                deployment
            )
        );
    }

    function little64(uint64 value) internal pure returns (bytes8 result) {
        uint64 reversed;
        for (uint8 i; i < 8; ++i) {
            reversed = (reversed << 8) | (value & 0xff);
            value >>= 8;
        }
        return bytes8(reversed);
    }
}
