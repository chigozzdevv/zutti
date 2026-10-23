// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";
import {SpvJournal} from "./SpvJournal.sol";

contract SpvRegistry {
    error InvalidConfiguration();
    error InvalidTransition();
    error StaleSource();
    error ObservationPending();
    error SourceHalted();

    struct Configuration {
        IRiscZeroVerifier verifier;
        bytes32 image;
        bytes32 policy;
        uint32 confirmations;
        uint32 minimumHeight;
        uint256 minimumWork;
        uint32 observationPeriod;
        uint32 maximumAge;
        uint32 futureDrift;
    }

    IRiscZeroVerifier public immutable verifier;
    bytes32 public immutable imageId;
    bytes32 public immutable policy;
    uint32 public immutable confirmations;
    uint32 public immutable minimumHeight;
    uint256 public immutable minimumWork;
    uint32 public immutable observationPeriod;
    uint32 public immutable maximumAge;
    uint32 public immutable futureDrift;

    bytes32 public stablePrefix;
    bytes32 public stableHash;
    uint32 public stableHeight;
    uint32 public headTimestamp;
    uint256 public cumulativeWork;
    uint64 public proposedAt;
    bool public halted;
    SpvJournal.Claim private _pending;
    mapping(bytes32 => bool) private _accepted;

    event Proposed(bytes32 indexed head, bytes32 indexed stable, uint256 work, uint64 observedAt);
    event Advanced(bytes32 indexed stable, uint32 height, uint256 headWork);
    event Conflict(bytes32 indexed stable, bytes32 indexed alternative, uint256 headWork);

    constructor(Configuration memory config) {
        if (
            address(config.verifier).code.length == 0 || config.image == 0 || config.policy == 0
                || config.confirmations == 0 || config.confirmations > 1000
                || config.minimumHeight == 0 || config.minimumWork == 0
                || config.observationPeriod == 0 || config.observationPeriod > 7 days
                || config.maximumAge < config.observationPeriod || config.maximumAge > 7 days
                || config.futureDrift > 2 hours
        ) revert InvalidConfiguration();
        verifier = config.verifier;
        imageId = config.image;
        policy = config.policy;
        confirmations = config.confirmations;
        minimumHeight = config.minimumHeight;
        minimumWork = config.minimumWork;
        observationPeriod = config.observationPeriod;
        maximumAge = config.maximumAge;
        futureDrift = config.futureDrift;
    }

    function pending() external view returns (SpvJournal.Claim memory) {
        return _pending;
    }

    function accepted(bytes32 checkpoint) external view returns (bool) {
        return !halted && _accepted[checkpoint];
    }

    function fresh() external view returns (bool) {
        return
            !halted && stablePrefix != 0 && block.timestamp <= uint256(headTimestamp) + maximumAge;
    }

    function submit(bytes calldata seal, SpvJournal.Claim calldata claim) external {
        if (halted) revert SourceHalted();
        _check(claim);
        if (
            claim.kind != 1 || claim.previous != stablePrefix || claim.stable.height <= stableHeight
                || claim.stable.prefix == stablePrefix
                || uint256(claim.head.height) - claim.stable.height != confirmations
                || claim.head.work <= _bestWork()
        ) revert InvalidTransition();
        _fresh(claim);
        verifier.verify{gas: 1_000_000}(seal, imageId, SpvJournal.digest(claim));
        _pending = claim;
        proposedAt = uint64(block.timestamp);
        emit Proposed(claim.head.prefix, claim.stable.prefix, claim.head.work, proposedAt);
    }

    function finalize() external {
        if (halted) revert SourceHalted();
        SpvJournal.Claim memory claim = _pending;
        if (claim.head.prefix == 0 || claim.previous != stablePrefix) revert InvalidTransition();
        if (block.timestamp < uint256(proposedAt) + observationPeriod) revert ObservationPending();
        _fresh(claim);
        stablePrefix = claim.stable.prefix;
        stableHash = claim.stable.hash;
        stableHeight = claim.stable.height;
        headTimestamp = claim.head.timestamp;
        cumulativeWork = claim.head.work;
        _accepted[stablePrefix] = true;
        delete _pending;
        proposedAt = 0;
        emit Advanced(stablePrefix, stableHeight, cumulativeWork);
    }

    function reportConflict(bytes calldata seal, SpvJournal.Claim calldata claim) external {
        if (halted) revert SourceHalted();
        _check(claim);
        if (
            claim.kind != 2 || stablePrefix == 0 || claim.previous != stablePrefix
                || claim.stable.height != stableHeight || claim.stable.prefix == stablePrefix
                || claim.stable.hash == stableHash
                || uint256(claim.head.height) - claim.stable.height < confirmations
                || claim.head.work <= _bestWork()
        ) revert InvalidTransition();
        verifier.verify{gas: 1_000_000}(seal, imageId, SpvJournal.digest(claim));
        halted = true;
        delete _pending;
        proposedAt = 0;
        emit Conflict(stablePrefix, claim.stable.prefix, claim.head.work);
    }

    function _bestWork() private view returns (uint256) {
        return _pending.head.work > cumulativeWork ? _pending.head.work : cumulativeWork;
    }

    function _check(SpvJournal.Claim memory claim) private view {
        if (
            claim.policy != policy || claim.stable.prefix == 0 || claim.stable.hash == 0
                || claim.head.prefix == 0 || claim.head.hash == 0
                || claim.stable.prefix == claim.head.prefix
                || claim.stable.height >= claim.head.height || claim.head.height < minimumHeight
                || claim.stable.work == 0 || claim.head.work <= claim.stable.work
                || claim.head.work < minimumWork || claim.maximumTime < claim.head.timestamp
                || claim.maximumTime < claim.stable.timestamp
                || uint256(claim.maximumTime) > block.timestamp + futureDrift
        ) revert InvalidTransition();
    }

    function _fresh(SpvJournal.Claim memory claim) private view {
        if (block.timestamp > uint256(claim.head.timestamp) + maximumAge) revert StaleSource();
    }
}
