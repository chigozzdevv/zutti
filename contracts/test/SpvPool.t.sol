// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SpvPool, ISpvToken} from "../src/SpvPool.sol";
import {SpvRegistry} from "../src/SpvRegistry.sol";
import {SpvJournal} from "../src/SpvJournal.sol";
import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";
import {TokenMock, JournalVerifierMock} from "./PrivatePool.t.sol";

interface SpvPoolVm {
    function warp(uint256 time) external;
    function load(address target, bytes32 slot) external view returns (bytes32);
    function expectRevert() external;
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
}

contract SpvPoolTest {
    SpvPoolVm private constant vm =
        SpvPoolVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 private constant POOL_IMAGE = bytes32(uint256(101));
    bytes32 private constant SOURCE_IMAGE = bytes32(uint256(102));
    bytes32 private constant BINDING_IMAGE = bytes32(uint256(103));
    JournalVerifierMock private verifier;
    TokenMock private token;
    SpvRegistry private source;
    SpvPool private pool;

    function setUp() public {
        vm.warp(1_800_000_000);
        verifier = new JournalVerifierMock();
        token = new TokenMock();
        source = new SpvRegistry(
            SpvRegistry.Configuration(
                IRiscZeroVerifier(address(verifier)), SOURCE_IMAGE, id(50), 2, 1, 1, 10, 3600, 60
            )
        );
        pool = new SpvPool(
            SpvPool.Configuration(
                ISpvToken(address(token)),
                IRiscZeroVerifier(address(verifier)),
                source,
                POOL_IMAGE,
                BINDING_IMAGE,
                3,
                id(51)
            )
        );
        token.mint(address(this), 300_000_000);
        token.approve(address(pool), 300_000_000);
        advance();
    }

    function id(uint256 value) private pure returns (bytes32) {
        return bytes32(value);
    }

    function advance() private {
        SpvJournal.Claim memory claim = SpvJournal.Claim(
            1,
            source.policy(),
            bytes32(0),
            SpvJournal.Point(id(10), 100, id(11), uint32(block.timestamp - 30), 100),
            SpvJournal.Point(id(12), 102, id(13), uint32(block.timestamp - 20), 102),
            uint32(block.timestamp - 20)
        );
        source.submit(abi.encode(SOURCE_IMAGE, SpvJournal.digest(claim)), claim);
        vm.warp(block.timestamp + 10);
        source.finalize();
    }

    function seal(SpvPool.Operation memory op) private view returns (bytes memory) {
        return abi.encode(POOL_IMAGE, pool.journalDigest(op));
    }

    function operation(uint8 kind, bytes32 input, bytes32 output)
        private
        view
        returns (SpvPool.Operation memory op)
    {
        op.kind = kind;
        op.owner = address(0);
        op.value = 0;
        op.checkpoint = 0;
        op.expiry = 0;
        op.orderTag = 0;
        op.order = 0;
        op.reservation = 0;
        op.root = kind == 1 ? bytes32(0) : pool.root();
        op.nullifiers = new bytes32[](kind == 1 ? 0 : 1);
        if (kind != 1) op.nullifiers[0] = input;
        op.outputs = new SpvPool.Output[](1);
        op.outputs[0] = SpvPool.Output(output, hex"123456");
        op.value = kind == 1 ? 300_000_000 : 0;
        op.owner = kind == 1 ? address(this) : address(0);
    }

    function reject(SpvPool.Operation memory op, bytes4 reason) private {
        bytes memory proof = seal(op);
        vm.expectRevert(reason);
        pool.execute(proof, op);
    }

    function deposit() private {
        SpvPool.Operation memory op = operation(1, 0, id(1));
        pool.execute(seal(op), op);
    }

    function reservation() private view returns (SpvPool.Operation memory op) {
        op = operation(4, id(2), id(3));
        op.checkpoint = id(10);
        op.expiry = 121;
        op.orderTag = id(4);
        op.order = id(5);
        op.reservation = id(6);
    }

    function fund() private {
        deposit();
        SpvPool.Operation memory op = reservation();
        pool.execute(seal(op), op);
    }

    function terminal(bytes32 output) private view returns (SpvPool.Operation memory op) {
        op = operation(5, id(99), output);
        op.checkpoint = id(10);
    }

    function testBackedReservationAndSingleTerminalNullifier() public {
        fund();
        require(pool.backing() == 300_000_000);
        SpvPool.Operation memory paid = terminal(id(7));
        pool.execute(seal(paid), paid);
        SpvPool.Operation memory refund = terminal(id(8));
        reject(refund, SpvPool.AlreadySpent.selector);
        require(pool.nextLeaf() == 3);
        require(pool.spent(id(99)));
    }

    function testDelayedClaimHasNoDestinationTimerAndNeedsNoMaker() public {
        fund();
        vm.warp(block.timestamp + 100 days);
        SpvPool.Operation memory paid = terminal(id(7));
        bytes memory proof = seal(paid);
        vm.prank(address(0xbeef));
        pool.execute(proof, paid);
        require(pool.spent(id(99)));
    }

    function testInvalidOrChangedProofCannotChooseAnotherOutput() public {
        fund();
        SpvPool.Operation memory paid = terminal(id(7));
        bytes memory proof = seal(paid);
        paid.outputs[0].commitment = id(8);
        vm.expectRevert();
        pool.execute(proof, paid);
        require(!pool.spent(id(99)));
        vm.expectRevert();
        pool.execute("", paid);
    }

    function testWrongCheckpointAndUnfundedRootFail() public {
        SpvPool.Operation memory op = terminal(id(7));
        op.root = id(999);
        reject(op, SpvPool.UnknownRoot.selector);
        fund();
        op = terminal(id(7));
        op.checkpoint = id(999);
        reject(op, SpvPool.UnknownCheckpoint.selector);
    }

    function testFundingRequiresFreshSourceAndRemainingInclusionTime() public {
        deposit();
        SpvPool.Operation memory op = reservation();
        op.expiry = 102;
        reject(op, SpvPool.InvalidOperation.selector);
        op.expiry = 121;
        vm.warp(block.timestamp + 3601);
        reject(op, SpvPool.InvalidOperation.selector);
        require(!pool.orders(op.orderTag));
    }

    function testRemainingWindowIncludesTheObservedConfirmationHead() public {
        deposit();
        SpvPool.Operation memory op = reservation();
        op.expiry = 104;
        reject(op, SpvPool.InvalidOperation.selector);
        op.expiry = 105;
        pool.execute(seal(op), op);
        require(pool.orders(op.orderTag));
    }

    function testStorageSlotsMatchTheEthereumReservationProofVerifier() public {
        fund();
        require(vm.load(address(pool), keccak256(abi.encode(id(3), uint256(70)))) == id(1));
        require(vm.load(address(pool), keccak256(abi.encode(id(4), uint256(71)))) == id(1));
        require(vm.load(address(pool), keccak256(abi.encode(id(99), uint256(69)))) == 0);
        SpvPool.Operation memory op = terminal(id(7));
        pool.execute(seal(op), op);
        require(vm.load(address(pool), keccak256(abi.encode(id(99), uint256(69)))) == id(1));
    }

    function testExpiryMutationIsBoundAndCannotBecomeASettlementTimer() public {
        deposit();
        SpvPool.Operation memory op = reservation();
        bytes memory proof = seal(op);
        op.expiry = 122;
        vm.expectRevert();
        pool.execute(proof, op);
        op = terminal(id(7));
        op.expiry = 121;
        reject(op, SpvPool.InvalidOperation.selector);
    }

    function testDuplicateReservationAndSourceHaltCannotAuthorizeAnotherSpend() public {
        fund();
        SpvPool.Operation memory op = reservation();
        op.nullifiers[0] = id(30);
        op.outputs[0].commitment = id(31);
        reject(op, SpvPool.AlreadySpent.selector);
        SpvJournal.Claim memory conflict = SpvJournal.Claim(
            2,
            source.policy(),
            id(10),
            SpvJournal.Point(id(40), 100, id(41), uint32(block.timestamp - 30), 110),
            SpvJournal.Point(id(42), 102, id(43), uint32(block.timestamp - 20), 120),
            uint32(block.timestamp - 20)
        );
        source.reportConflict(abi.encode(SOURCE_IMAGE, SpvJournal.digest(conflict)), conflict);
        op = terminal(id(7));
        reject(op, SpvPool.UnknownCheckpoint.selector);
    }

    function testFeeTokenAndInvalidProofRollBackBackingAndTree() public {
        SpvPool.Operation memory op = operation(1, 0, id(1));
        token.configure(true, false);
        reject(op, SpvPool.TokenTransfer.selector);
        require(pool.backing() == 0 && pool.nextLeaf() == 0);
        token.configure(false, false);
        vm.expectRevert();
        pool.execute("", op);
        require(pool.backing() == 0 && pool.nextLeaf() == 0);
    }
}
