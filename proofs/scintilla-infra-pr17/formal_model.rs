//! Bounded model checker for regional single-writer failover.
//! The model enforces one active writer, only on a healthy region, and requires
//! a writerless handoff before health mutation or promotion to another region.

use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Region {
    A,
    B,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct State {
    writer: Option<Region>,
    a_healthy: bool,
    b_healthy: bool,
    epoch: u8,
}

fn healthy(state: State, region: Region) -> bool {
    return match region {
        Region::A => state.a_healthy,
        Region::B => state.b_healthy,
    };
}

fn invariant(state: State) -> bool {
    let writer_is_healthy = state
        .writer
        .map(|region| healthy(state, region))
        .unwrap_or(true);
    let writer_has_epoch = state.writer.is_none() || state.epoch > 0;
    let epoch_is_bounded = state.epoch <= 3;

    return writer_is_healthy && writer_has_epoch && epoch_is_bounded;
}

fn next(state: State) -> Vec<State> {
    let mut next_states = Vec::new();

    if state.writer.is_none() {
        for region in [Region::A, Region::B] {
            if healthy(state, region) && state.epoch < 3 {
                next_states.push(State {
                    writer: Some(region),
                    epoch: state.epoch + 1,
                    ..state
                });
            }
        }

        next_states.push(State {
            a_healthy: !state.a_healthy,
            ..state
        });
        next_states.push(State {
            b_healthy: !state.b_healthy,
            ..state
        });
    } else {
        next_states.push(State {
            writer: None,
            ..state
        });
    }

    return next_states;
}

fn main() {
    let initial = State {
        writer: None,
        a_healthy: true,
        b_healthy: true,
        epoch: 0,
    };
    let mut seen = HashSet::from([initial]);
    let mut queue = VecDeque::from([initial]);

    while let Some(state) = queue.pop_front() {
        assert!(invariant(state), "invalid failover state: {state:?}");

        for candidate in next(state) {
            assert!(
                invariant(candidate),
                "unsafe failover transition: {state:?} -> {candidate:?}"
            );
            assert!(candidate.epoch >= state.epoch, "writer epoch regressed");

            if seen.insert(candidate) {
                queue.push_back(candidate);
            }
        }
    }

    assert!(seen.iter().any(|state| { state.writer == Some(Region::A) }));
    assert!(seen.iter().any(|state| { state.writer == Some(Region::B) }));

    println!(
        "scintilla-infra formal model: explored {} states; invariants hold",
        seen.len()
    );
}
