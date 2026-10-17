// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PoolJournal} from "../src/PoolJournal.sol";

interface JournalVm {
    function readFile(string calldata path) external view returns (string memory);
    function projectRoot() external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key) external pure returns (string memory);
    function parseBytes32(string calldata value) external pure returns (bytes32);
}

contract PoolJournalTest {
    JournalVm private constant vm = JournalVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testRustAndSolidityBindTheSameDeploymentAndPoolJournal() public view {
        string memory v = vm.readFile(string.concat(vm.projectRoot(), "/../tests/fixtures/pool.json"));
        bytes32 domain = PoolJournal.deploymentDomain(
            repeat(17),
            repeat(85),
            repeat(102),
            repeat(119),
            31337,
            address(bytes20(repeat(51))),
            address(bytes20(repeat(68)))
        );
        require(domain == digest(v, ".domain"));
        PoolJournal.Claim memory claim = PoolJournal.Claim(
            2,
            domain,
            repeat(34),
            0,
            0,
            0,
            0,
            0,
            address(0),
            digest(v, ".nullifiers_digest"),
            digest(v, ".outputs_digest")
        );
        require(PoolJournal.digest(claim) == digest(v, ".pool_digest"));
        bytes32 source = sha256(
            abi.encodePacked(
                bytes8("ZUTTISRC"),
                hex"0100",
                repeat(17),
                repeat(85),
                bytes32(0),
                bytes32(0),
                repeat(170),
                repeat(187),
                uint256(42)
            )
        );
        require(source == digest(v, ".source_digest"));
    }

    function digest(string memory json, string memory path) private pure returns (bytes32) {
        return vm.parseBytes32(string.concat("0x", vm.parseJsonString(json, path)));
    }

    function repeat(uint8 value) private pure returns (bytes32 result) {
        for (uint8 i; i < 32; ++i) {
            result |= bytes32(uint256(value) << (8 * i));
        }
    }
}
