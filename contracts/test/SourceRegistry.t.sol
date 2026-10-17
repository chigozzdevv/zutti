// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SourceRegistry} from "../src/SourceRegistry.sol";
import {RiscZeroGroth16Verifier} from "../vendor/risc0/groth16/RiscZeroGroth16Verifier.sol";
import {ControlID} from "../vendor/risc0/groth16/ControlID.sol";

interface SourceVm {
    function envOr(string calldata name, string calldata fallbackValue) external returns (string memory);
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key) external pure returns (string memory);
    function parseBytes(string calldata value) external pure returns (bytes memory);
    function expectRevert() external;
    function skip(bool skipTest) external;
}

contract SourceRegistryTest {
    SourceVm private constant vm = SourceVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testGenuineRecursiveSourceUpdateAndTampering() public {
        string memory directory = vm.envOr("ZUTTI_SOURCE_JOB_DIR", string(""));
        string memory job;
        string memory claim;
        string memory result;
        if (bytes(directory).length == 0) {
            result = vm.readFile("../tests/fixtures/source-update-proof.json");
            job = result;
            claim = result;
        } else {
            job = vm.readFile(string.concat(directory, "/job.json"));
            claim = vm.readFile(string.concat(directory, "/claim.json"));
            result = vm.readFile(string.concat(directory, "/result.json"));
        }
        RiscZeroGroth16Verifier verifier =
            new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
        SourceRegistry source = new SourceRegistry(
            verifier, readWord(job, ".image_id"), readWord(claim, ".genesis"), readWord(claim, ".policy"), 1
        );
        bytes memory seal = readBytes(result, ".seal");
        bytes32 state = readWord(claim, ".state");
        bytes32 checkpoint = readWord(claim, ".checkpoint");
        uint256 work = uint256(readWord(claim, ".work_be"));
        vm.expectRevert();
        source.update(seal, bytes32(0), bytes32(uint256(state) ^ 1), checkpoint, work);
        SourceRegistry belowFloor = new SourceRegistry(
            verifier, readWord(job, ".image_id"), readWord(claim, ".genesis"), readWord(claim, ".policy"), work + 1
        );
        vm.expectRevert();
        belowFloor.update(seal, bytes32(0), state, checkpoint, work);
        require(belowFloor.headState() == bytes32(0));
        source.update(seal, bytes32(0), state, checkpoint, work);
        require(source.headState() == state);
        require(source.stableCheckpoint() == checkpoint);
        require(source.accepted(checkpoint));
        require(source.cumulativeWork() == work);
        vm.expectRevert();
        source.update(seal, bytes32(0), state, checkpoint, work);
        SourceRegistry fresh = new SourceRegistry(
            verifier, readWord(job, ".image_id"), readWord(claim, ".genesis"), readWord(claim, ".policy"), 1
        );
        seal[seal.length - 1] = bytes1(uint8(seal[seal.length - 1]) ^ 1);
        vm.expectRevert();
        fresh.update(seal, bytes32(0), state, checkpoint, work);
        require(fresh.headState() == bytes32(0));
    }

    function readWord(string memory json, string memory key) private pure returns (bytes32) {
        return bytes32(readBytes(json, key));
    }

    function readBytes(string memory json, string memory key) private pure returns (bytes memory) {
        return vm.parseBytes(string.concat("0x", vm.parseJsonString(json, key)));
    }
}
