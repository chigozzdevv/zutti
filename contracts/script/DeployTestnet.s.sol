// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";
import {SpvRegistry} from "../src/SpvRegistry.sol";
import {SpvPool, ISpvToken} from "../src/SpvPool.sol";

interface TestnetVm {
    function envString(string calldata name) external returns (string memory);
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key)
        external
        pure
        returns (string memory);
    function parseJsonUint(string calldata json, string calldata key)
        external
        pure
        returns (uint256);
    function parseJsonBool(string calldata json, string calldata key) external pure returns (bool);
    function parseBytes32(string calldata value) external pure returns (bytes32);
    function startBroadcast() external;
    function stopBroadcast() external;
}

contract DeployTestnet {
    TestnetVm private constant vm =
        TestnetVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address public constant VERIFIER = 0xB121B667dd2cf27F95f9F5107137696F56f188f6;
    address public constant USDC = 0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238;

    function word(string memory profile, string memory key) private pure returns (bytes32) {
        return vm.parseBytes32(string.concat("0x", vm.parseJsonString(profile, key)));
    }

    function number(string memory profile, string memory key) private pure returns (uint32) {
        uint256 value = vm.parseJsonUint(profile, key);
        require(value <= type(uint32).max, "Configuration value is too large");
        return uint32(value);
    }

    function run() external returns (SpvRegistry source, SpvPool pool) {
        require(block.chainid == 11155111, "Sepolia only");
        string memory profile = vm.readFile(vm.envString("ZUTTI_PROFILE"));
        require(vm.parseJsonBool(profile, ".deployment_allowed"), "Profile is not deployable");
        require(
            keccak256(bytes(vm.parseJsonString(profile, ".network"))) == keccak256("testnet")
                && vm.parseJsonUint(profile, ".policy.version") == 2
                && !vm.parseJsonBool(profile, ".nonpayment_refunds"),
            "Unsupported source profile"
        );
        require(VERIFIER.code.length != 0 && USDC.code.length != 0, "Missing pinned contracts");
        bytes32 sourcePolicy = word(profile, ".policy.id");
        uint32 checkpointHeight = number(profile, ".checkpoint.height");
        require(
            checkpointHeight < number(profile, ".policy.valid_through"), "Expired source horizon"
        );
        SpvRegistry.Configuration memory config = SpvRegistry.Configuration({
            verifier: IRiscZeroVerifier(VERIFIER),
            image: word(profile, ".programs.admission.image_id"),
            policy: sourcePolicy,
            confirmations: number(profile, ".policy.confirmations"),
            minimumHeight: checkpointHeight + 1,
            minimumWork: uint256(word(profile, ".checkpoint.work")),
            observationPeriod: number(profile, ".registry.observation_period"),
            maximumAge: number(profile, ".registry.maximum_age"),
            futureDrift: number(profile, ".registry.future_drift")
        });
        uint32 minimumRemaining = number(profile, ".minimum_remaining");
        bytes32 poolImage = word(profile, ".programs.pool.image_id");
        bytes32 bindingImage = word(profile, ".programs.binding.image_id");
        bytes32 proofProfile = word(profile, ".proof_profile_id");
        vm.startBroadcast();
        source = new SpvRegistry(config);
        pool = new SpvPool(
            SpvPool.Configuration(
                ISpvToken(USDC),
                IRiscZeroVerifier(VERIFIER),
                source,
                poolImage,
                bindingImage,
                minimumRemaining,
                proofProfile
            )
        );
        vm.stopBroadcast();
    }
}
