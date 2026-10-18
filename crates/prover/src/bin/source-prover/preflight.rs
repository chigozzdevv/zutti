use zutti_prover::{Error, local::ExecutionMetrics};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) fn fit(
    blocks: &mut Vec<Vec<u8>>,
    ceiling: u64,
    mut execute: impl FnMut(&[Vec<u8>]) -> Result<ExecutionMetrics>,
) -> Result<ExecutionMetrics> {
    if blocks.is_empty() {
        return Err("source preflight requires at least one block".into());
    }
    loop {
        match execute(blocks) {
            Ok(metrics) if metrics.cycles > 0 && metrics.cycles <= ceiling => return Ok(metrics),
            Ok(metrics) if metrics.cycles == 0 => {
                return Err("source preflight returned no execution cycles".into());
            }
            Ok(_) => {}
            Err(error) if error.downcast_ref::<Error>() == Some(&Error::ExecutionLimit) => {}
            Err(error) => return Err(error),
        }
        if blocks.len() == 1 {
            return Err(
                "a single source block exceeds the configured proving cycle ceiling".into(),
            );
        }
        blocks.truncate(blocks.len().div_ceil(2));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn metrics(cycles: u64) -> ExecutionMetrics {
        ExecutionMetrics {
            cycles,
            segments: 1,
            seconds: 0.0,
            journal_matches: true,
            proof_generated: false,
        }
    }

    #[test]
    fn capacity_failures_reduce_the_same_consecutive_prefix() {
        let mut blocks = (1..=9).map(|n| vec![n]).collect::<Vec<_>>();
        let mut attempts = Vec::new();
        let measured = fit(&mut blocks, 100, |batch| {
            attempts.push(batch.to_vec());
            if batch.len() > 5 {
                Err(Error::ExecutionLimit.into())
            } else {
                Ok(metrics(batch.len() as u64 * 30))
            }
        })
        .unwrap();
        assert_eq!(attempts.iter().map(Vec::len).collect::<Vec<_>>(), [9, 5, 3]);
        assert_eq!(blocks, [vec![1], vec![2], vec![3]]);
        assert_eq!(measured.cycles, 90);
    }

    #[test]
    fn invalid_execution_is_not_retried_as_a_smaller_batch() {
        let mut blocks = vec![vec![1], vec![2]];
        let mut attempts = 0;
        let error = fit(&mut blocks, 100, |_| {
            attempts += 1;
            Err(Error::Journal.into())
        })
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Error>(), Some(&Error::Journal));
        assert_eq!(attempts, 1);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn single_block_capacity_failure_stops_without_discarding_the_block() {
        for hard in [false, true] {
            let mut blocks = vec![vec![1]];
            assert!(
                fit(&mut blocks, 100, |_| {
                    if hard {
                        Err(Error::ExecutionLimit.into())
                    } else {
                        Ok(metrics(101))
                    }
                })
                .is_err()
            );
            assert_eq!(blocks, [vec![1]]);
        }
    }
}
