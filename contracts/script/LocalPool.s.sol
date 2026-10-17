// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PrivatePool, IERC20} from "../src/PrivatePool.sol";
import {SourceRegistry} from "../src/SourceRegistry.sol";
import {RiscZeroGroth16Verifier} from "../vendor/risc0/groth16/RiscZeroGroth16Verifier.sol";
import {ControlID} from "../vendor/risc0/groth16/ControlID.sol";

interface LocalVm {
    function envAddress(string calldata key) external returns (address);
    function envBytes32(string calldata key) external returns (bytes32);
    function startBroadcast(address sender) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata objectKey, string calldata key, address value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata key, uint256 value)
        external
        returns (string memory);
    function serializeBytes32(string calldata objectKey, string calldata key, bytes32 value)
        external
        returns (string memory);
    function writeJson(string calldata value, string calldata path) external;
}

contract LocalDemoToken {
    string public constant name = "Local demo USDC";
    string public constant symbol = "USDC";
    uint8 public constant decimals = 6;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address owner, uint256 value) external {
        require(block.chainid == 31337);
        balanceOf[owner] += value;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        return true;
    }

    function transfer(address receiver, uint256 value) external returns (bool) {
        balanceOf[msg.sender] -= value;
        balanceOf[receiver] += value;
        return true;
    }

    function transferFrom(address owner, address receiver, uint256 value) external returns (bool) {
        allowance[owner][msg.sender] -= value;
        balanceOf[owner] -= value;
        balanceOf[receiver] += value;
        return true;
    }
}

contract LocalPoolScript {
    LocalVm private constant vm = LocalVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function run() external {
        require(block.chainid == 31337);
        address owner = vm.envAddress("ZUTTI_LOCAL_OWNER");
        bytes32 poolImage = vm.envBytes32("ZUTTI_POOL_IMAGE");
        bytes32 preparationImage = vm.envBytes32("ZUTTI_PREPARATION_IMAGE");
        bytes32 sourceImage = vm.envBytes32("ZUTTI_SOURCE_UPDATE_IMAGE");
        bytes32 genesis = vm.envBytes32("ZUTTI_SOURCE_GENESIS");
        bytes32 policy = vm.envBytes32("ZUTTI_SOURCE_POLICY");
        bytes32 paymentProfile = bytes32(uint256(0x66) * (type(uint256).max / 255));
        bytes32 proofProfile = bytes32(uint256(0x77) * (type(uint256).max / 255));
        vm.startBroadcast(owner);
        RiscZeroGroth16Verifier verifier =
            new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
        SourceRegistry source = new SourceRegistry(verifier, sourceImage, genesis, policy, 1);
        LocalDemoToken token = new LocalDemoToken();
        PrivatePool pool = new PrivatePool(
            PrivatePool.Configuration(
                IERC20(address(token)), verifier, source, poolImage, preparationImage, paymentProfile, proofProfile
            )
        );
        token.mint(owner, 200_000_000);
        token.approve(address(pool), 200_000_000);
        vm.stopBroadcast();
        vm.serializeUint("deployment", "chain_id", block.chainid);
        vm.serializeAddress("deployment", "owner", owner);
        vm.serializeAddress("deployment", "verifier", address(verifier));
        vm.serializeAddress("deployment", "source", address(source));
        vm.serializeAddress("deployment", "token", address(token));
        vm.serializeAddress("deployment", "pool", address(pool));
        vm.serializeBytes32("deployment", "source_genesis", genesis);
        vm.serializeBytes32("deployment", "source_policy", policy);
        vm.serializeBytes32("deployment", "payment_profile", paymentProfile);
        vm.serializeBytes32("deployment", "proof_profile", proofProfile);
        vm.serializeBytes32("deployment", "pool_image", poolImage);
        string memory data = vm.serializeBytes32("deployment", "domain", pool.domain());
        vm.writeJson(data, "../.data/local-e2e/deployment.json");
    }
}
