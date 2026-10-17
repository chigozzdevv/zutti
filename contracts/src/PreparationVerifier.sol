// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";

contract PreparationVerifier {
    uint256 public constant VERIFICATION_GAS = 1_000_000;
    error InvalidConfiguration();
    error InvalidCommitment();

    IRiscZeroVerifier public immutable verifier;
    bytes32 public immutable imageId;
    bytes32 public immutable domain;

    constructor(IRiscZeroVerifier verifier_, bytes32 imageId_, bytes32 domain_) {
        if (address(verifier_).code.length == 0 || imageId_ == bytes32(0) || domain_ == bytes32(0)) {
            revert InvalidConfiguration();
        }
        verifier = verifier_;
        imageId = imageId_;
        domain = domain_;
    }

    function verify(bytes calldata seal, bytes32 order, bytes32 reservation) external view {
        if (order == bytes32(0) || reservation == bytes32(0)) revert InvalidCommitment();
        bytes32 journalDigest = sha256(abi.encodePacked(bytes8("ZUTTIPRP"), hex"0100", domain, order, reservation));
        verifier.verify{gas: VERIFICATION_GAS}(seal, imageId, journalDigest);
    }
}
