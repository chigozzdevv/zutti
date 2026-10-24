// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;
import {PoolJournal} from "../src/PoolJournal.sol";
import {SpvPoolJournal} from "../src/SpvPoolJournal.sol";

interface SpvJournalVm {
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata data, string calldata path)
        external
        pure
        returns (string memory);
    function parseJsonUint(string calldata data, string calldata path)
        external
        pure
        returns (uint256);
    function parseBytes32(string calldata data) external pure returns (bytes32);
}

contract SpvPoolJournalTest {
    SpvJournalVm private constant vm =
        SpvJournalVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function word(string memory data, string memory path) private pure returns (bytes32) {
        return vm.parseBytes32(string.concat("0x", vm.parseJsonString(data, path)));
    }

    function testRustAndSolidityMatchFundingPayoutAndRefundJournals() public view {
        string memory data = vm.readFile("../tests/fixtures/spv/pool-journal.json");
        string[3] memory names = [string("reservation"), string("paid"), string("unpaid")];
        for (uint256 i; i < 3; i++) {
            string memory p = string.concat(".", names[i], ".");
            PoolJournal.Claim memory claim = PoolJournal.Claim(
                uint8(vm.parseJsonUint(data, string.concat(p, "kind"))),
                word(data, string.concat(p, "domain")),
                word(data, string.concat(p, "root")),
                word(data, string.concat(p, "checkpoint")),
                word(data, string.concat(p, "order_tag")),
                word(data, string.concat(p, "order")),
                word(data, string.concat(p, "reservation")),
                0,
                address(0),
                word(data, string.concat(p, "nullifiers")),
                word(data, string.concat(p, "outputs"))
            );
            bytes32 digest = SpvPoolJournal.digest(
                word(data, string.concat(p, "policy")),
                word(data, string.concat(p, "binding_image")),
                word(data, string.concat(p, "pool_image")),
                uint32(vm.parseJsonUint(data, string.concat(p, "expiry"))),
                claim
            );
            require(
                digest == word(data, string.concat(p, "journal_digest")),
                "Rust/SPV pool journal mismatch"
            );
        }
    }
}
