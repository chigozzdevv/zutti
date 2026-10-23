// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SpvRegistry} from "../src/SpvRegistry.sol";
import {SpvJournal} from "../src/SpvJournal.sol";
import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";

interface SpvVm {
    function warp(uint256 timestamp) external;
    function expectRevert(bytes4 selector) external;
    function expectRevert() external;
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonString(string calldata json, string calldata key)
        external
        pure
        returns (string memory);
    function parseJsonUint(string calldata json, string calldata key)
        external
        pure
        returns (uint256);
    function parseBytes(string calldata value) external pure returns (bytes memory);
}

contract SpvVerifierMock {
    function verify(bytes calldata seal, bytes32 image, bytes32 journal) external pure {
        (bytes32 expectedImage, bytes32 expectedJournal) = abi.decode(seal, (bytes32, bytes32));
        require(image == expectedImage && journal == expectedJournal, "invalid test evidence");
    }
}

contract SpvRegistryTest {
    SpvVm private constant vm = SpvVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 private constant IMAGE = bytes32(uint256(1));
    bytes32 private constant POLICY = bytes32(uint256(2));
    SpvRegistry private source;

    function setUp() public {
        vm.warp(1000);
        source = deploy(POLICY, IMAGE, 1, 300);
    }

    function testCannotAcceptBeforeObservationAndAcceptsWithoutAnOperator() public {
        SpvJournal.Claim memory claim = initial();
        source.submit(seal(claim), claim);
        require(!source.accepted(claim.stable.prefix));
        vm.expectRevert(SpvRegistry.ObservationPending.selector);
        source.finalize();
        vm.warp(1060);
        source.finalize();
        require(source.stablePrefix() == claim.stable.prefix);
        require(source.stableHash() == claim.stable.hash);
        require(source.accepted(claim.stable.prefix));
        require(source.cumulativeWork() == claim.head.work);
        require(source.fresh());
        vm.expectRevert(SpvRegistry.InvalidTransition.selector);
        source.finalize();
    }

    function testHeavierCompetingCandidateResetsObservationAndRejectsOldWork() public {
        SpvJournal.Claim memory first = initial();
        source.submit(seal(first), first);
        vm.warp(1030);
        SpvJournal.Claim memory heavier = initial();
        heavier.stable.prefix = word(40);
        heavier.stable.hash = word(41);
        heavier.stable.work = 350;
        heavier.head.prefix = word(42);
        heavier.head.hash = word(43);
        heavier.head.work = 450;
        source.submit(seal(heavier), heavier);
        expectSubmit(first, SpvRegistry.InvalidTransition.selector);
        vm.warp(1060);
        vm.expectRevert(SpvRegistry.ObservationPending.selector);
        source.finalize();
        vm.warp(1090);
        source.finalize();
        require(source.accepted(heavier.stable.prefix));
        require(!source.accepted(first.stable.prefix));
    }

    function testStaleCandidatesAndExpiredObservationCannotAdvance() public {
        SpvJournal.Claim memory claim = initial();
        claim.head.timestamp = 699;
        expectSubmit(claim, SpvRegistry.StaleSource.selector);
        claim = initial();
        source.submit(seal(claim), claim);
        vm.warp(1301);
        vm.expectRevert(SpvRegistry.StaleSource.selector);
        source.finalize();
        require(source.stablePrefix() == 0);
    }

    function testFutureTimestampAnywhereInThePrefixIsRejected() public {
        SpvJournal.Claim memory claim = initial();
        claim.maximumTime = 1031;
        expectSubmit(claim, SpvRegistry.InvalidTransition.selector);
        require(source.proposedAt() == 0);
    }

    function testPolicyDepthFloorParentAndProofMutationsAreRejected() public {
        SpvJournal.Claim memory claim = initial();
        bytes memory proof = seal(claim);
        claim.policy = word(30);
        vm.expectRevert(SpvRegistry.InvalidTransition.selector);
        source.submit(proof, claim);
        claim = initial();
        claim.head.height++;
        expectSubmit(claim, SpvRegistry.InvalidTransition.selector);
        claim = initial();
        claim.head.work = 250;
        expectSubmit(claim, SpvRegistry.InvalidTransition.selector);
        claim = initial();
        claim.previous = word(88);
        expectSubmit(claim, SpvRegistry.InvalidTransition.selector);
        claim = initial();
        claim.stable.hash = word(99);
        vm.expectRevert();
        source.submit(proof, claim);
        require(source.stablePrefix() == 0 && source.proposedAt() == 0);
    }

    function testUpdatesMustExtendTheExactAcceptedStablePrefix() public {
        SpvJournal.Claim memory first = initial();
        accept(first);
        SpvJournal.Claim memory next = initial();
        next.previous = first.stable.prefix;
        next.stable = SpvJournal.Point(word(40), 4, word(41), 1060, 400);
        next.head = SpvJournal.Point(word(42), 5, word(43), 1060, 500);
        next.maximumTime = 1060;
        bytes memory proof = seal(next);
        next.previous = word(50);
        expectSubmit(next, SpvRegistry.InvalidTransition.selector);
        next.previous = first.stable.prefix;
        source.submit(proof, next);
        vm.warp(1120);
        source.finalize();
        require(source.accepted(first.stable.prefix));
        require(source.accepted(next.stable.prefix));
    }

    function testHeavierStableConflictHaltsAdmissionAndInvalidatesAcceptedLookup() public {
        SpvJournal.Claim memory first = initial();
        accept(first);
        SpvJournal.Claim memory conflict = initial();
        conflict.kind = 2;
        conflict.previous = first.stable.prefix;
        conflict.stable.prefix = word(40);
        conflict.stable.hash = word(41);
        conflict.stable.work = 350;
        conflict.head.prefix = word(42);
        conflict.head.hash = word(43);
        conflict.head.work = 450;
        source.reportConflict(seal(conflict), conflict);
        require(source.halted());
        require(!source.accepted(first.stable.prefix));
        require(!source.fresh());
        vm.expectRevert(SpvRegistry.SourceHalted.selector);
        source.finalize();
        expectSubmit(first, SpvRegistry.SourceHalted.selector);
    }

    function testConflictCannotUseSameStableHashWrongHeightOrLesserWork() public {
        SpvJournal.Claim memory first = initial();
        accept(first);
        SpvJournal.Claim memory conflict = initial();
        conflict.kind = 2;
        conflict.previous = first.stable.prefix;
        conflict.stable.prefix = word(40);
        conflict.head.work = 450;
        expectConflict(conflict, SpvRegistry.InvalidTransition.selector);
        conflict.stable.hash = word(41);
        conflict.stable.height--;
        expectConflict(conflict, SpvRegistry.InvalidTransition.selector);
        conflict.stable.height = first.stable.height;
        conflict.head.work = first.head.work;
        expectConflict(conflict, SpvRegistry.InvalidTransition.selector);
        require(!source.halted());
    }

    function testRustGuestJournalMatchesSolidityEncodingWithMockEvidenceOnly() public {
        string memory fixture = vm.readFile("../tests/fixtures/spv/admission-journal.json");
        SpvJournal.Claim memory claim;
        claim.kind = uint8(vm.parseJsonUint(fixture, ".kind"));
        claim.policy = readWord(fixture, ".policy");
        claim.previous = readWord(fixture, ".previous");
        claim.stable = readPoint(fixture, ".stable");
        claim.head = readPoint(fixture, ".head");
        claim.maximumTime = uint32(vm.parseJsonUint(fixture, ".maximum_time"));
        bytes memory journal =
            vm.parseBytes(string.concat("0x", vm.parseJsonString(fixture, ".journal")));
        require(keccak256(SpvJournal.journal(claim)) == keccak256(journal));
        require(SpvJournal.digest(claim) == readWord(fixture, ".journal_sha256"));
        SpvRegistry registry = deploy(claim.policy, readWord(fixture, ".image_id"), 1, 1);
        vm.warp(claim.maximumTime);
        bytes memory proof = abi.encode(readWord(fixture, ".image_id"), SpvJournal.digest(claim));
        registry.submit(proof, claim);
        vm.warp(block.timestamp + 60);
        registry.finalize();
        require(registry.accepted(claim.stable.prefix));
    }

    function testInvalidConfigurationIsRejected() public {
        SpvRegistry.Configuration memory config = configuration(POLICY, IMAGE, 1, 1);
        config.observationPeriod = 0;
        vm.expectRevert(SpvRegistry.InvalidConfiguration.selector);
        new SpvRegistry(config);
        config = configuration(POLICY, IMAGE, 1, 1);
        config.futureDrift = 7201;
        vm.expectRevert(SpvRegistry.InvalidConfiguration.selector);
        new SpvRegistry(config);
    }

    function deploy(bytes32 policy, bytes32 image, uint32 depth, uint256 work)
        private
        returns (SpvRegistry)
    {
        return new SpvRegistry(configuration(policy, image, depth, work));
    }

    function configuration(bytes32 policy, bytes32 image, uint32 depth, uint256 work)
        private
        returns (SpvRegistry.Configuration memory)
    {
        return SpvRegistry.Configuration(
            IRiscZeroVerifier(address(new SpvVerifierMock())),
            image,
            policy,
            depth,
            1,
            work,
            60,
            300,
            30
        );
    }

    function initial() private pure returns (SpvJournal.Claim memory) {
        return SpvJournal.Claim(
            1,
            POLICY,
            bytes32(0),
            SpvJournal.Point(word(10), 3, word(11), 999, 300),
            SpvJournal.Point(word(12), 4, word(13), 1000, 400),
            1000
        );
    }

    function accept(SpvJournal.Claim memory claim) private {
        source.submit(seal(claim), claim);
        vm.warp(block.timestamp + 60);
        source.finalize();
    }

    function readPoint(string memory json, string memory path)
        private
        pure
        returns (SpvJournal.Point memory)
    {
        return SpvJournal.Point(
            readWord(json, string.concat(path, ".prefix")),
            uint32(vm.parseJsonUint(json, string.concat(path, ".height"))),
            readWord(json, string.concat(path, ".hash")),
            uint32(vm.parseJsonUint(json, string.concat(path, ".timestamp"))),
            uint256(readWord(json, string.concat(path, ".work")))
        );
    }

    function readWord(string memory json, string memory key) private pure returns (bytes32) {
        return bytes32(vm.parseBytes(string.concat("0x", vm.parseJsonString(json, key))));
    }

    function expectSubmit(SpvJournal.Claim memory claim, bytes4 selector) private {
        bytes memory proof = seal(claim);
        vm.expectRevert(selector);
        source.submit(proof, claim);
    }

    function expectConflict(SpvJournal.Claim memory claim, bytes4 selector) private {
        bytes memory proof = seal(claim);
        vm.expectRevert(selector);
        source.reportConflict(proof, claim);
    }

    function seal(SpvJournal.Claim memory claim) private pure returns (bytes memory) {
        return abi.encode(IMAGE, SpvJournal.digest(claim));
    }

    function word(uint256 value) private pure returns (bytes32) {
        return bytes32(value);
    }
}
