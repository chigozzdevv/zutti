// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PrivatePool, IERC20} from "../src/PrivatePool.sol";
import {SourceRegistry} from "../src/SourceRegistry.sol";
import {RiscZeroGroth16Verifier} from "../vendor/risc0/groth16/RiscZeroGroth16Verifier.sol";
import {ControlID} from "../vendor/risc0/groth16/ControlID.sol";
import {LocalDemoToken} from "../script/LocalPool.s.sol";

interface LifecycleVm {
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key) external pure returns (string memory);
    function parseJsonUint(string calldata json, string calldata key) external pure returns (uint256);
    function parseBytes(string calldata value) external pure returns (bytes memory);
    function parseAddress(string calldata value) external pure returns (address);
    function startPrank(address sender) external;
    function stopPrank() external;
    function setNonce(address owner, uint64 nonce) external;
    function chainId(uint256 value) external;
    function expectRevert() external;
}

contract PoolLifecycleTest {
    LifecycleVm private constant vm = LifecycleVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testGenuineDepositTransferWithdrawalAndReplayProtection() public {
        vm.chainId(31337);
        string memory data = vm.readFile("../tests/fixtures/pool-lifecycle.json");
        address owner = vm.parseAddress(vm.parseJsonString(data, ".deployment.owner"));
        vm.setNonce(owner, 0);
        vm.startPrank(owner);
        RiscZeroGroth16Verifier verifier =
            new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
        SourceRegistry source = new SourceRegistry(
            verifier,
            readWord(vm.readFile("../tests/fixtures/source-update-proof.json"), ".image_id"),
            readWord(data, ".deployment.source_genesis"),
            readWord(data, ".deployment.source_policy"),
            1
        );
        LocalDemoToken token = new LocalDemoToken();
        PrivatePool pool = new PrivatePool(
            PrivatePool.Configuration(
                IERC20(address(token)),
                verifier,
                source,
                readWord(data, ".deployment.pool_image"),
                0xf97aeb93b4066460d372b433a89bd07d6be30934a55fa5de6a5e4c421d9b3916,
                readWord(data, ".deployment.payment_profile"),
                readWord(data, ".deployment.proof_profile")
            )
        );
        require(address(pool) == vm.parseAddress(vm.parseJsonString(data, ".deployment.pool")));
        require(address(token) == vm.parseAddress(vm.parseJsonString(data, ".deployment.token")));
        require(pool.domain() == readWord(data, ".deployment.domain"));
        token.mint(owner, 200_000_000);
        token.approve(address(pool), 200_000_000);
        execute(pool, data, "deposit", 1);
        require(pool.backing() == 200_000_000);
        execute(pool, data, "transfer", 2);
        require(pool.backing() == 200_000_000);
        execute(pool, data, "withdraw", 1);
        require(pool.backing() == 100_000_000);
        require(token.balanceOf(address(pool)) == 100_000_000);
        require(token.balanceOf(owner) == 100_000_000);
        require(pool.nextLeaf() == 4);
        vm.stopPrank();
    }

    function execute(PrivatePool pool, string memory data, string memory name, uint256 outputCount) private {
        string memory prefix = string.concat(".operations.", name);
        uint8 kind = uint8(vm.parseJsonUint(data, string.concat(prefix, ".kind")));
        bytes32[] memory nullifiers = new bytes32[](kind == 1 ? 0 : 1);
        if (kind != 1) nullifiers[0] = readWord(data, string.concat(prefix, ".nullifiers[0]"));
        PrivatePool.Output[] memory outputs = new PrivatePool.Output[](outputCount);
        for (uint256 i; i < outputCount; ++i) {
            string memory index = i == 0 ? "0" : "1";
            string memory key = string.concat(prefix, ".outputs[", index, "]");
            outputs[i] = PrivatePool.Output(
                readWord(data, string.concat(key, ".commitment")), readBytes(data, string.concat(key, ".ciphertext"))
            );
        }
        PrivatePool.Operation memory op = PrivatePool.Operation(
            kind,
            readWord(data, string.concat(prefix, ".root")),
            bytes32(0),
            bytes32(0),
            bytes32(0),
            bytes32(0),
            uint64(vm.parseJsonUint(data, string.concat(prefix, ".value"))),
            vm.parseAddress(string.concat("0x", vm.parseJsonString(data, string.concat(prefix, ".owner")))),
            nullifiers,
            outputs
        );
        bytes memory seal = readBytes(data, string.concat(prefix, ".seal"));
        pool.execute(seal, op, "");
        vm.expectRevert();
        pool.execute(seal, op, "");
    }

    function readWord(string memory json, string memory key) private pure returns (bytes32) {
        return bytes32(readBytes(json, key));
    }

    function readBytes(string memory json, string memory key) private pure returns (bytes memory) {
        string memory value = vm.parseJsonString(json, key);
        bytes memory encoded = bytes(value);
        if (encoded.length >= 2 && encoded[0] == "0" && encoded[1] == "x") return vm.parseBytes(value);
        return vm.parseBytes(string.concat("0x", value));
    }
}
