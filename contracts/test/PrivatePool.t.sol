// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {PrivatePool, IERC20} from "../src/PrivatePool.sol";
import {SourceRegistry} from "../src/SourceRegistry.sol";
import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";

interface PoolVm {
    function expectRevert() external;
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
    function warp(uint256 time) external;
}

contract JournalVerifierMock {
    function verify(bytes calldata seal, bytes32 image, bytes32 journal) external pure {
        require(keccak256(seal) == keccak256(abi.encode(image, journal)), "mock claim mismatch");
    }
}

contract TokenMock is IERC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    bool public fee;
    bool public fail;
    address public callback;
    bytes public callbackData;

    function mint(address owner, uint256 value) external {
        balanceOf[owner] += value;
    }

    function approve(address spender, uint256 value) external {
        allowance[msg.sender][spender] = value;
    }

    function configure(bool fee_, bool fail_) external {
        fee = fee_;
        fail = fail_;
    }

    function hook(address target, bytes calldata data) external {
        callback = target;
        callbackData = data;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        move(msg.sender, to, value);
        return !fail;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        allowance[from][msg.sender] -= value;
        if (callback != address(0)) {
            (bool success,) = callback.call(callbackData);
            require(!success, "reentrancy succeeded");
        }
        move(from, to, value);
        return !fail;
    }

    function move(address from, address to, uint256 value) private {
        balanceOf[from] -= value;
        balanceOf[to] += fee ? value - 1 : value;
    }
}

contract PrivatePoolTest {
    PoolVm private constant vm = PoolVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 private constant POOL_IMAGE = bytes32(uint256(101));
    bytes32 private constant SOURCE_IMAGE = bytes32(uint256(102));
    bytes32 private constant PREPARATION_IMAGE = bytes32(uint256(103));
    JournalVerifierMock private verifier;
    TokenMock private token;
    SourceRegistry private source;
    PrivatePool private pool;

    function setUp() public {
        verifier = new JournalVerifierMock();
        token = new TokenMock();
        source = new SourceRegistry(IRiscZeroVerifier(address(verifier)), SOURCE_IMAGE, id(1), id(2), 1);
        pool = new PrivatePool(
            PrivatePool.Configuration({
                token: token,
                verifier: IRiscZeroVerifier(address(verifier)),
                source: source,
                poolImage: POOL_IMAGE,
                preparationImage: PREPARATION_IMAGE,
                paymentProfile: id(3),
                proofProfile: id(4)
            })
        );
        token.mint(address(this), 1_000_000_000);
        token.approve(address(pool), type(uint256).max);
    }

    function testDepositTracksBackingWhilePrivateOperationsKeepTokenBalance() public {
        deposit(id(10), 200_000_000);
        require(pool.backing() == 200_000_000 && token.balanceOf(address(pool)) == 200_000_000);
        PrivatePool.Operation memory op = operation(2, pool.root(), id(20), id(11));
        execute(op);
        require(pool.spent(id(20)) && pool.commitments(id(11)) && pool.nextLeaf() == 2);
        require(pool.backing() == 200_000_000 && token.balanceOf(address(pool)) == 200_000_000);
    }

    function testInvalidReceiptCannotMoveTokensOrAppendLeaves() public {
        PrivatePool.Operation memory op = depositOperation(id(10), 200_000_000);
        bytes32 beforeRoot = pool.root();
        vm.expectRevert();
        pool.execute(abi.encode(POOL_IMAGE, id(99)), op, "");
        require(pool.root() == beforeRoot && pool.nextLeaf() == 0 && pool.backing() == 0);
        require(token.balanceOf(address(pool)) == 0);
    }

    function testFeeTokenAndFalseReturnRollBackAllPoolState() public {
        PrivatePool.Operation memory op = depositOperation(id(10), 200_000_000);
        bytes32 beforeRoot = pool.root();
        token.configure(true, false);
        expectExecutionRevert(op, PrivatePool.TokenTransfer.selector);
        require(pool.root() == beforeRoot && pool.nextLeaf() == 0 && !pool.commitments(id(10)));
        require(token.balanceOf(address(pool)) == 0);
        token.configure(false, true);
        expectExecutionRevert(op, PrivatePool.TokenTransfer.selector);
        require(pool.backing() == 0 && token.balanceOf(address(pool)) == 0);
    }

    function testDuplicateNullifierAndCommitmentCannotCreateExtraValue() public {
        deposit(id(10), 200_000_000);
        PrivatePool.Operation memory op = operation(2, pool.root(), id(20), id(11));
        op.nullifiers = new bytes32[](2);
        op.nullifiers[0] = id(20);
        op.nullifiers[1] = id(20);
        expectExecutionRevert(op, PrivatePool.AlreadySpent.selector);
        require(!pool.spent(id(20)) && pool.nextLeaf() == 1);
        op.nullifiers = new bytes32[](1);
        op.nullifiers[0] = id(20);
        op.outputs[0].commitment = id(10);
        expectExecutionRevert(op, PrivatePool.DuplicateCommitment.selector);
        require(!pool.spent(id(20)) && pool.nextLeaf() == 1);
    }

    function testWithdrawalBindsDestinationAndRejectsReplay() public {
        deposit(id(10), 200_000_000);
        PrivatePool.Operation memory op = operation(3, pool.root(), id(20), bytes32(0));
        op.value = 200_000_000;
        op.owner = address(0x1234);
        bytes memory seal = abi.encode(POOL_IMAGE, pool.journalDigest(op));
        op.owner = address(0x5678);
        vm.expectRevert();
        pool.execute(seal, op, "");
        op.owner = address(0x1234);
        vm.prank(address(0x9999));
        pool.execute(seal, op, "");
        require(token.balanceOf(address(0x1234)) == 200_000_000 && pool.backing() == 0);
        vm.expectRevert(PrivatePool.AlreadySpent.selector);
        pool.execute(seal, op, "");
    }

    function testUnknownPoolAndSourceRootsAreRejected() public {
        PrivatePool.Operation memory op = operation(2, id(99), id(20), id(11));
        expectExecutionRevert(op, PrivatePool.UnknownRoot.selector);
        deposit(id(10), 200_000_000);
        op = operation(5, pool.root(), id(20), id(11));
        op.checkpoint = id(50);
        expectExecutionRevert(op, PrivatePool.UnknownCheckpoint.selector);
    }

    function testReservationRequiresPreparationAndSingleOrderUse() public {
        deposit(id(10), 200_000_000);
        acceptSource(id(30), id(50), 1);
        PrivatePool.Operation memory op = reservation(id(20), id(11));
        bytes memory reserveSeal = abi.encode(POOL_IMAGE, pool.journalDigest(op));
        vm.expectRevert();
        pool.execute(reserveSeal, op, hex"00");
        execute(op);
        require(pool.orders(id(60)) && pool.commitments(id(11)) && pool.backing() == 200_000_000);
        op.root = pool.root();
        op.nullifiers[0] = id(21);
        op.outputs[0].commitment = id(12);
        expectExecutionRevert(op, PrivatePool.AlreadySpent.selector);
        require(!pool.spent(id(21)));
    }

    function testTerminalTransitionConsumesOneReservationAndHasNoClaimTimer() public {
        deposit(id(10), 200_000_000);
        acceptSource(id(30), id(50), 1);
        execute(reservation(id(20), id(11)));
        PrivatePool.Operation memory op = operation(5, pool.root(), id(21), id(12));
        op.checkpoint = id(50);
        vm.warp(block.timestamp + 365 days);
        execute(op);
        require(pool.spent(id(21)) && pool.commitments(id(12)) && pool.backing() == 200_000_000);
        op.outputs[0].commitment = id(13);
        op.root = pool.root();
        expectExecutionRevert(op, PrivatePool.AlreadySpent.selector);
        require(!pool.commitments(id(13)));
    }

    function testHistoricalMembershipRootsRemainUsableWithFreshNullifiers() public {
        deposit(id(10), 200_000_000);
        bytes32 previous = pool.root();
        deposit(id(11), 100_000_000);
        execute(operation(2, previous, id(20), id(12)));
        require(pool.roots(previous) && pool.commitments(id(12)));
    }

    function testReentrantTokenCannotExecuteAnotherOperation() public {
        PrivatePool.Operation memory op = depositOperation(id(10), 200_000_000);
        bytes memory data =
            abi.encodeCall(PrivatePool.execute, (abi.encode(POOL_IMAGE, pool.journalDigest(op)), op, bytes("")));
        token.hook(address(pool), data);
        execute(op);
        require(pool.nextLeaf() == 1 && pool.backing() == 200_000_000);
    }

    function testSourceUpdatesBindPreviousStateStablePrefixAndIncreasingWork() public {
        acceptSource(id(30), id(50), 1);
        acceptSource(id(31), id(51), 2);
        require(source.accepted(id(50)) && source.accepted(id(51)) && source.headState() == id(31));
        vm.expectRevert(SourceRegistry.InvalidTransition.selector);
        source.update("", id(30), id(32), id(52), 3);
        vm.expectRevert(SourceRegistry.InvalidTransition.selector);
        source.update("", id(31), id(32), id(52), 2);
        vm.expectRevert();
        source.update(abi.encode(SOURCE_IMAGE, id(99)), id(31), id(32), id(52), 3);
        require(!source.accepted(id(52)) && source.cumulativeWork() == 2);
    }

    function testPoolDomainSeparatesDeploymentsAndReceipts() public {
        PrivatePool other = new PrivatePool(
            PrivatePool.Configuration({
                token: token,
                verifier: IRiscZeroVerifier(address(verifier)),
                source: source,
                poolImage: POOL_IMAGE,
                preparationImage: PREPARATION_IMAGE,
                paymentProfile: id(3),
                proofProfile: id(4)
            })
        );
        require(pool.domain() != other.domain());
        PrivatePool.Operation memory op = depositOperation(id(10), 200_000_000);
        bytes memory seal = abi.encode(POOL_IMAGE, pool.journalDigest(op));
        vm.expectRevert();
        other.execute(seal, op, "");
    }

    function deposit(bytes32 commitment, uint64 value) private {
        execute(depositOperation(commitment, value));
    }

    function depositOperation(bytes32 commitment, uint64 value) private view returns (PrivatePool.Operation memory op) {
        op = operation(1, bytes32(0), bytes32(0), commitment);
        op.value = value;
        op.owner = address(this);
    }

    function operation(uint8 kind, bytes32 root, bytes32 nullifier, bytes32 commitment)
        private
        pure
        returns (PrivatePool.Operation memory op)
    {
        op.kind = kind;
        op.root = root;
        op.nullifiers = new bytes32[](nullifier == 0 ? 0 : 1);
        if (nullifier != 0) op.nullifiers[0] = nullifier;
        op.outputs = new PrivatePool.Output[](commitment == 0 ? 0 : 1);
        if (commitment != 0) op.outputs[0] = PrivatePool.Output(commitment, hex"aabb");
    }

    function reservation(bytes32 nullifier, bytes32 commitment) private view returns (PrivatePool.Operation memory op) {
        op = operation(4, pool.root(), nullifier, commitment);
        op.checkpoint = id(50);
        op.orderTag = id(60);
        op.order = id(61);
        op.reservation = id(62);
    }

    function expectExecutionRevert(PrivatePool.Operation memory op, bytes4 selector) private {
        (bytes memory seal, bytes memory prep) = proofs(op);
        vm.expectRevert(selector);
        pool.execute(seal, op, prep);
    }

    function execute(PrivatePool.Operation memory op) private {
        (bytes memory seal, bytes memory prep) = proofs(op);
        pool.execute(seal, op, prep);
    }

    function proofs(PrivatePool.Operation memory op) private view returns (bytes memory seal, bytes memory prep) {
        if (op.kind == 4) {
            prep = abi.encode(
                PREPARATION_IMAGE,
                sha256(abi.encodePacked(bytes8("ZUTTIPRP"), hex"0100", pool.domain(), op.order, op.reservation))
            );
        }
        seal = abi.encode(POOL_IMAGE, pool.journalDigest(op));
    }

    function acceptSource(bytes32 state, bytes32 checkpoint, uint256 work) private {
        bytes32 previous = source.headState();
        bytes32 digest = sha256(
            abi.encodePacked(
                bytes8("ZUTTISRC"),
                hex"0100",
                source.genesis(),
                source.policy(),
                previous,
                source.stableCheckpoint(),
                state,
                checkpoint,
                work
            )
        );
        source.update(abi.encode(SOURCE_IMAGE, digest), previous, state, checkpoint, work);
    }

    function id(uint256 value) private pure returns (bytes32) {
        return bytes32(value);
    }
}
