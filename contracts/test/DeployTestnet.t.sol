// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {DeployTestnet} from "../script/DeployTestnet.s.sol";

interface DeploymentVm {
    function chainId(uint256 chain) external;
    function setEnv(string calldata name, string calldata value) external;
    function expectRevert(bytes calldata reason) external;
    function projectRoot() external view returns (string memory);
}

contract DeployTestnetTest {
    DeploymentVm private constant vm =
        DeploymentVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testRejectsMainnetBeforeReadingConfigurationOrBroadcasting() external {
        vm.chainId(1);
        DeployTestnet deployment = new DeployTestnet();
        vm.expectRevert(abi.encodeWithSignature("Error(string)", "Sepolia only"));
        deployment.run();
    }

    function testRejectsRetiredProfileBeforeBroadcasting() external {
        vm.chainId(11155111);
        vm.setEnv(
            "ZUTTI_PROFILE",
            string.concat(vm.projectRoot(), "/../deploy/source/checkpoint/profile.json")
        );
        DeployTestnet deployment = new DeployTestnet();
        vm.expectRevert(abi.encodeWithSignature("Error(string)", "Profile is not deployable"));
        deployment.run();
    }
}
