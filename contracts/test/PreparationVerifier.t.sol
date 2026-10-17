// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PreparationVerifier} from "../src/PreparationVerifier.sol";
import {RiscZeroGroth16Verifier} from "../vendor/risc0/groth16/RiscZeroGroth16Verifier.sol";
import {ControlID} from "../vendor/risc0/groth16/ControlID.sol";

interface Vm {
    function envOr(string calldata name, string calldata fallbackValue) external returns (string memory);
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key) external pure returns (string memory);
    function parseBytes(string calldata value) external pure returns (bytes memory);
    function expectRevert() external;
    function expectRevert(bytes4 selector) external;
    function skip(bool skipTest) external;
}

contract PreparationVerifierTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    RiscZeroGroth16Verifier private verifier;

    function setUp() public {
        verifier = new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
    }

    function testRejectsUnprovedPreparation() public {
        PreparationVerifier adapter = new PreparationVerifier(verifier, bytes32(uint256(1)), bytes32(uint256(2)));
        vm.expectRevert();
        adapter.verify(new bytes(260), bytes32(uint256(3)), bytes32(uint256(4)));
    }

    function testRejectsEmptyCommitments() public {
        PreparationVerifier adapter = new PreparationVerifier(verifier, bytes32(uint256(1)), bytes32(uint256(2)));
        vm.expectRevert(PreparationVerifier.InvalidCommitment.selector);
        adapter.verify(new bytes(260), bytes32(0), bytes32(uint256(4)));
    }

    function testRejectsVerifierWithoutCode() public {
        vm.expectRevert(PreparationVerifier.InvalidConfiguration.selector);
        new PreparationVerifier(RiscZeroGroth16Verifier(address(1)), bytes32(uint256(1)), bytes32(uint256(2)));
    }

    function testRealBoundlessPreparationAndTampering() public {
        string memory directory = vm.envOr("ZUTTI_PUBLIC_JOB_DIR", string(""));
        if (bytes(directory).length == 0) {
            vm.skip(true);
            return;
        }
        string memory claim = vm.readFile(string.concat(directory, "/claim.json"));
        string memory job = vm.readFile(string.concat(directory, "/job.json"));
        string memory result = vm.readFile(string.concat(directory, "/result.json"));
        bytes32 image = bytes32(readHex(job, ".image_id"));
        bytes32 domain = bytes32(readHex(claim, ".domain"));
        bytes32 order = bytes32(readHex(claim, ".order"));
        bytes32 reservation = bytes32(readHex(claim, ".reservation"));
        bytes memory seal = readHex(result, ".seal");
        PreparationVerifier adapter = new PreparationVerifier(verifier, image, domain);
        adapter.verify(seal, order, reservation);
        vm.expectRevert();
        adapter.verify(seal, bytes32(uint256(order) ^ 1), reservation);
        PreparationVerifier wrongDomain = new PreparationVerifier(verifier, image, bytes32(uint256(domain) ^ 1));
        vm.expectRevert();
        wrongDomain.verify(seal, order, reservation);
        seal[seal.length - 1] = bytes1(uint8(seal[seal.length - 1]) ^ 1);
        vm.expectRevert();
        adapter.verify(seal, order, reservation);
    }

    function readHex(string memory json, string memory key) private pure returns (bytes memory) {
        return vm.parseBytes(string.concat("0x", vm.parseJsonString(json, key)));
    }
}
