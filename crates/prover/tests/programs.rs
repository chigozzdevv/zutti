#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::encoding::Canonical;
use zutti_prover::{
    program::{Program, ProgramKind},
    source::{MAX_SOURCE_CYCLES, SourceJob, SourcePrograms},
};
use zutti_source::proof::{SourceInput, SourceWitness};

#[test]
fn archived_programs_preserve_their_identity_and_reject_wrong_kinds_or_changed_elfs() {
    for kind in [
        ProgramKind::Pool,
        ProgramKind::Preparation,
        ProgramKind::HistoryQuery,
    ] {
        let original = Program::current(kind);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("program");
        original.export_new(&path).unwrap();
        assert!(original.export_new(&path).is_err());
        let restored = Program::read(kind, &path.join("program.elf")).unwrap();
        assert_eq!(restored.image(), original.image());
        assert_eq!(restored.bytes(), original.bytes());
        assert!(Program::from_bytes(ProgramKind::Source, original.bytes().to_vec()).is_err());
        let mut altered = original.bytes().to_vec();
        altered[0] ^= 1;
        assert!(Program::from_bytes(kind, altered).is_err());
    }
}

#[test]
fn source_jobs_resume_from_archived_coherent_programs_and_detect_elf_substitution() {
    let directory = tempfile::tempdir().unwrap();
    for kind in [ProgramKind::Source, ProgramKind::SourceUpdate] {
        Program::current(kind)
            .export_new(&directory.path().join(kind.name()))
            .unwrap();
    }
    let programs = SourcePrograms::read(directory.path()).unwrap();
    let policy = programs.policy(NetworkType::Main, 0, 3);
    let genesis = zebra_test::vectors::MAINNET_BLOCKS[&0].to_vec();
    let witness = SourceWitness {
        policy,
        input: SourceInput::Genesis(genesis),
    };
    let job = SourceJob::transition_with(&witness, None, &programs).unwrap();
    for cycles in [0, zutti_prover::source::MAX_DIAGNOSTIC_CYCLES + 1] {
        let error = zutti_prover::query::LocalProver::new("must-not-run")
            .compare_source_execution_with_limit(
                &job,
                std::path::Path::new("missing-candidate"),
                None,
                cycles,
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<zutti_prover::Error>(),
            Some(&zutti_prover::Error::Binding)
        );
    }
    for cycles in [0, MAX_SOURCE_CYCLES + 1] {
        let path = directory.path().join(format!("rejected-{cycles}"));
        assert!(job.export_public(&path, cycles).is_err());
        assert!(!path.exists());
    }
    let maximum = directory.path().join("maximum");
    job.export_public(&maximum, MAX_SOURCE_CYCLES).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(maximum.join("job.json")).unwrap()).unwrap();
    assert_eq!(manifest["cycles"], MAX_SOURCE_CYCLES);
    let path = directory.path().join("job");
    job.export_public(&path, 268_435_456).unwrap();
    let restored = SourceJob::read_public(&path).unwrap();
    assert_eq!(
        restored.statement().to_bytes().unwrap(),
        job.statement().to_bytes().unwrap()
    );
    let mut changed = std::fs::read(path.join("stdin.bin")).unwrap();
    changed[5] ^= 1;
    std::fs::write(path.join("stdin.bin"), changed).unwrap();
    assert!(!restored.matches(&SourceJob::read_public(&path).unwrap()));
    std::fs::write(
        path.join("program.elf"),
        Program::current(ProgramKind::Pool).bytes(),
    )
    .unwrap();
    assert!(SourceJob::read_public(&path).is_err());
    assert!(
        SourcePrograms::new(
            Program::current(ProgramKind::Pool),
            Program::current(ProgramKind::SourceUpdate)
        )
        .is_err()
    );
}
