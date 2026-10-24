// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PoolJournal} from "./PoolJournal.sol";
import {SpvJournal} from "./SpvJournal.sol";

library SpvPoolJournal {
    function digest(
        bytes32 policy,
        bytes32 bindingImage,
        uint32 expiry,
        PoolJournal.Claim memory claim
    ) internal pure returns (bytes32) {
        return sha256(
            abi.encodePacked(
                bytes8("ZUTTISPC"),
                hex"0100",
                policy,
                bindingImage,
                SpvJournal.little32(expiry),
                PoolJournal.journal(claim)
            )
        );
    }
}
