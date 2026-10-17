// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";

contract SourceRegistry {
    error InvalidConfiguration();
    error InvalidTransition();

    IRiscZeroVerifier public immutable verifier;
    bytes32 public immutable imageId;
    bytes32 public immutable genesis;
    bytes32 public immutable policy;
    uint256 public immutable minimumWork;

    bytes32 public headState;
    bytes32 public stableCheckpoint;
    uint256 public cumulativeWork;
    mapping(bytes32 => bool) public accepted;

    event SourceUpdated(bytes32 indexed state, bytes32 indexed checkpoint, uint256 work);

    constructor(
        IRiscZeroVerifier verifier_,
        bytes32 imageId_,
        bytes32 genesis_,
        bytes32 policy_,
        uint256 minimumWork_
    ) {
        if (address(verifier_).code.length == 0 || imageId_ == 0 || genesis_ == 0 || policy_ == 0 || minimumWork_ == 0)
        {
            revert InvalidConfiguration();
        }
        verifier = verifier_;
        imageId = imageId_;
        genesis = genesis_;
        policy = policy_;
        minimumWork = minimumWork_;
    }

    function update(bytes calldata seal, bytes32 previous, bytes32 state, bytes32 checkpoint, uint256 work) external {
        if (
            previous != headState || state == 0 || state == previous || checkpoint == 0 || work <= cumulativeWork
                || work < minimumWork
        ) {
            revert InvalidTransition();
        }
        bytes32 digest = sha256(
            abi.encodePacked(
                bytes8("ZUTTISRC"), hex"0100", genesis, policy, previous, stableCheckpoint, state, checkpoint, work
            )
        );
        verifier.verify{gas: 1_000_000}(seal, imageId, digest);
        headState = state;
        stableCheckpoint = checkpoint;
        cumulativeWork = work;
        accepted[checkpoint] = true;
        emit SourceUpdated(state, checkpoint, work);
    }
}
