// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

library SpvJournal {
    struct Point {
        bytes32 prefix;
        uint32 height;
        bytes32 hash;
        uint32 timestamp;
        uint256 work;
    }

    struct Claim {
        uint8 kind;
        bytes32 policy;
        bytes32 previous;
        Point stable;
        Point head;
        uint32 maximumTime;
    }

    function point(Point memory value) internal pure returns (bytes memory) {
        return abi.encodePacked(
            value.prefix, little32(value.height), value.hash, little32(value.timestamp), value.work
        );
    }

    function journal(Claim memory claim) internal pure returns (bytes memory) {
        return abi.encodePacked(
            bytes8("ZUTTISPA"),
            hex"0100",
            claim.kind,
            claim.policy,
            claim.previous,
            point(claim.stable),
            point(claim.head),
            little32(claim.maximumTime)
        );
    }

    function digest(Claim memory claim) internal pure returns (bytes32) {
        return sha256(journal(claim));
    }

    function little32(uint32 value) internal pure returns (bytes4) {
        return bytes4(
            ((value & 0xff) << 24) | ((value & 0xff00) << 8) | ((value & 0xff0000) >> 8)
                | ((value & 0xff000000) >> 24)
        );
    }
}
