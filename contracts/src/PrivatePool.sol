// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IRiscZeroVerifier} from "../vendor/risc0/IRiscZeroVerifier.sol";
import {PreparationVerifier} from "./PreparationVerifier.sol";
import {SourceRegistry} from "./SourceRegistry.sol";
import {PoolJournal} from "./PoolJournal.sol";

interface IERC20 {
    function balanceOf(address owner) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

contract PrivatePool {
    struct Configuration {
        IERC20 token;
        IRiscZeroVerifier verifier;
        SourceRegistry source;
        bytes32 poolImage;
        bytes32 preparationImage;
        bytes32 paymentProfile;
        bytes32 proofProfile;
    }

    struct Output {
        bytes32 commitment;
        bytes ciphertext;
    }

    struct Operation {
        uint8 kind;
        bytes32 root;
        bytes32 checkpoint;
        bytes32 orderTag;
        bytes32 order;
        bytes32 reservation;
        uint64 value;
        address owner;
        bytes32[] nullifiers;
        Output[] outputs;
    }

    error InvalidConfiguration();
    error InvalidOperation();
    error UnknownRoot();
    error UnknownCheckpoint();
    error AlreadySpent();
    error DuplicateCommitment();
    error TokenTransfer();
    error Reentrancy();
    error TreeFull();

    IERC20 public immutable token;
    IRiscZeroVerifier public immutable verifier;
    SourceRegistry public immutable source;
    PreparationVerifier public immutable preparation;
    bytes32 public immutable imageId;
    bytes32 public immutable domain;

    uint64 public nextLeaf;
    uint256 public backing;
    bytes32 public root;
    bytes32[32] private frontier;
    bytes32[33] private empty;
    mapping(bytes32 => bool) public roots;
    mapping(bytes32 => bool) public spent;
    mapping(bytes32 => bool) public commitments;
    mapping(bytes32 => bool) public orders;
    bool private entered;

    event OutputInserted(uint32 indexed position, bytes32 indexed commitment, bytes ciphertext);
    event NullifierSpent(bytes32 indexed nullifier);

    constructor(Configuration memory config) {
        if (
            address(config.token).code.length == 0 || address(config.verifier).code.length == 0
                || address(config.source).code.length == 0 || config.poolImage == 0 || config.preparationImage == 0
                || config.paymentProfile == 0 || config.proofProfile == 0 || block.chainid > type(uint64).max
        ) {
            revert InvalidConfiguration();
        }
        token = config.token;
        verifier = config.verifier;
        source = config.source;
        imageId = config.poolImage;
        domain = PoolJournal.deploymentDomain(
            config.source.genesis(),
            config.source.policy(),
            config.paymentProfile,
            config.proofProfile,
            uint64(block.chainid),
            address(this),
            address(config.token)
        );
        preparation = new PreparationVerifier(config.verifier, config.preparationImage, domain);
        empty[0] = leafHash(bytes32(0));
        for (uint8 level; level < 32; ++level) {
            empty[level + 1] = nodeHash(level, empty[level], empty[level]);
        }
        root = empty[32];
        roots[root] = true;
    }

    function execute(bytes calldata seal, Operation calldata op, bytes calldata preparationSeal) external {
        if (entered) revert Reentrancy();
        entered = true;
        validate(op, preparationSeal);
        verifier.verify{gas: 1_000_000}(seal, imageId, journalDigest(op));
        for (uint256 i; i < op.nullifiers.length; ++i) {
            bytes32 nullifier = op.nullifiers[i];
            if (nullifier == 0 || spent[nullifier]) revert AlreadySpent();
            spent[nullifier] = true;
            emit NullifierSpent(nullifier);
        }
        if (op.kind == 4) {
            if (orders[op.orderTag]) revert AlreadySpent();
            orders[op.orderTag] = true;
        }
        for (uint256 i; i < op.outputs.length; ++i) {
            insert(op.outputs[i]);
        }
        if (op.kind == 1) {
            uint256 balance = token.balanceOf(address(this));
            callToken(abi.encodeCall(IERC20.transferFrom, (msg.sender, address(this), op.value)));
            if (token.balanceOf(address(this)) != balance + op.value) revert TokenTransfer();
            backing += op.value;
        } else if (op.kind == 3) {
            if (backing < op.value) revert TokenTransfer();
            backing -= op.value;
            uint256 balance = token.balanceOf(address(this));
            callToken(abi.encodeCall(IERC20.transfer, (op.owner, op.value)));
            if (token.balanceOf(address(this)) + op.value != balance) revert TokenTransfer();
        }
        entered = false;
    }

    function validate(Operation calldata op, bytes calldata preparationSeal) private view {
        uint256 inputs = op.nullifiers.length;
        uint256 outputs = op.outputs.length;
        if (op.kind == 0 || op.kind > 5 || inputs > 8 || outputs > 8) revert InvalidOperation();
        if (op.kind == 1) {
            if (inputs != 0 || outputs != 1 || op.value == 0 || op.owner != msg.sender || op.root != 0) {
                revert InvalidOperation();
            }
        } else {
            if (inputs == 0 || !roots[op.root]) revert UnknownRoot();
            if (op.kind == 3) {
                if (op.value == 0 || op.owner == address(0) || op.owner == address(this)) {
                    revert InvalidOperation();
                }
            } else if (op.value != 0 || op.owner != address(0) || outputs == 0) {
                revert InvalidOperation();
            }
        }
        if (op.kind == 4 || op.kind == 5) {
            if (!source.accepted(op.checkpoint)) revert UnknownCheckpoint();
        } else if (op.checkpoint != 0) {
            revert InvalidOperation();
        }
        if (op.kind == 4) {
            if (op.orderTag == 0 || op.order == 0 || op.reservation == 0) {
                revert InvalidOperation();
            }
            preparation.verify(preparationSeal, op.order, op.reservation);
        } else if (op.orderTag != 0 || op.order != 0 || op.reservation != 0 || preparationSeal.length != 0) {
            revert InvalidOperation();
        }
        if (op.kind == 5 && (inputs != 1 || outputs != 1)) revert InvalidOperation();
    }

    function journalDigest(Operation calldata op) public view returns (bytes32) {
        bytes memory nullifiers;
        for (uint256 i; i < op.nullifiers.length; ++i) {
            nullifiers = bytes.concat(nullifiers, op.nullifiers[i]);
        }
        bytes memory outputs;
        for (uint256 i; i < op.outputs.length; ++i) {
            outputs = bytes.concat(outputs, op.outputs[i].commitment, sha256(op.outputs[i].ciphertext));
        }
        return PoolJournal.digest(
            PoolJournal.Claim(
                op.kind,
                domain,
                op.root,
                op.checkpoint,
                op.orderTag,
                op.order,
                op.reservation,
                op.value,
                op.owner,
                sha256(nullifiers),
                sha256(outputs)
            )
        );
    }

    function insert(Output calldata output) private {
        if (nextLeaf >= uint64(1) << 32) revert TreeFull();
        if (output.commitment == 0 || output.ciphertext.length == 0 || output.ciphertext.length > 4096) {
            revert InvalidOperation();
        }
        if (commitments[output.commitment]) revert DuplicateCommitment();
        commitments[output.commitment] = true;
        uint32 position = uint32(nextLeaf++);
        uint32 index = position;
        bytes32 node = leafHash(output.commitment);
        for (uint8 level; level < 32; ++level) {
            if (index & 1 == 0) {
                frontier[level] = node;
                node = nodeHash(level, node, empty[level]);
            } else {
                node = nodeHash(level, frontier[level], node);
            }
            index >>= 1;
        }
        root = node;
        roots[node] = true;
        emit OutputInserted(position, output.commitment, output.ciphertext);
    }

    function leafHash(bytes32 commitment) private view returns (bytes32) {
        return sha256(abi.encodePacked(bytes8("ZUTTIPLF"), hex"0100", domain, commitment));
    }

    function nodeHash(uint8 level, bytes32 left, bytes32 right) private view returns (bytes32) {
        return sha256(abi.encodePacked(bytes8("ZUTTIPND"), hex"0100", domain, level, left, right));
    }

    function callToken(bytes memory data) private {
        (bool success, bytes memory result) = address(token).call(data);
        if (!success || (result.length != 0 && (result.length != 32 || !abi.decode(result, (bool))))) {
            revert TokenTransfer();
        }
    }
}
