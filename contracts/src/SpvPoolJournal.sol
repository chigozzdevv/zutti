// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PoolJournal} from "./PoolJournal.sol";
import {SpvJournal} from "./SpvJournal.sol";

library SpvPoolJournal {
    function digest(
        bytes32 policy,
        bytes32 bindingImage,
        bytes32 poolImage,
        uint32 expiry,
        PoolJournal.Claim memory claim
    ) internal pure returns (bytes32) {
        return sha256(
            abi.encodePacked(
                bytes8("ZUTTISPC"),
                hex"0200",
                policy,
                bindingImage,
                poolImage,
                SpvJournal.little32(expiry),
                PoolJournal.journal(claim)
            )
        );
    }
}
