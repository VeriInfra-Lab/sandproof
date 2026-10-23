use sandbox_controller_core::Store;
use sandproof_callback_state_extract::{retain_toggle, retain_trace, toggle_step, trace_step};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn store(rows: &[(u64, u64)]) -> Store<u64, u64> {
    let mut result = Store::default();
    for &(key, value) in rows {
        result.put(key, value);
    }
    result
}

#[test]
fn toggle_mutates_the_external_state_and_returns_it() {
    let mut state = false;
    assert!(toggle_step(&mut state, &0, &u64::MAX));
    assert!(state, "callback must write the original external state");
    assert!(!toggle_step(&mut state, &u64::MAX, &0));
    assert!(!state);
}

#[test]
fn trace_preserves_prefix_and_appends_exactly_one_full_row() {
    let mut visited = vec![(8, 9)];
    assert!(trace_step(&mut visited, &u64::MAX, &u64::MAX, 0));
    assert_eq!(visited, [(8, 9), (u64::MAX, u64::MAX)]);
    assert!(!trace_step(&mut visited, &0, &0, 0));
    assert_eq!(visited, [(8, 9), (u64::MAX, u64::MAX), (0, 0)]);
}

#[test]
fn empty_retain_does_not_touch_external_state() {
    let mut rows = Store::default();
    let mut state = true;
    retain_toggle(&mut rows, &mut state);
    assert!(state);
    let mut visited = vec![(8, 9)];
    retain_trace(&mut rows, &mut visited, 0);
    assert_eq!(visited, [(8, 9)]);
    assert!(rows.entries().is_empty());
}

#[test]
fn toggle_visits_each_original_position_and_keeps_in_order() {
    let mut rows = store(&[(1, 10), (7, 11), (2, 20)]);
    let mut state = false;
    retain_toggle(&mut rows, &mut state);
    assert!(state);
    assert_eq!(rows.entries(), [(1, 10), (2, 20)]);
}

#[test]
fn even_toggle_from_true_returns_true_and_keeps_even_positions() {
    let mut rows = store(&[(1, 10), (7, 11), (2, 20), (4, 40)]);
    let mut state = true;
    retain_toggle(&mut rows, &mut state);
    assert!(state);
    assert_eq!(rows.entries(), [(7, 11), (4, 40)]);
}

#[test]
fn trace_visits_rejected_rows_and_preserves_survivor_order() {
    let original = [(1, 10), (7, 11), (2, 12), (4, 20)];
    let mut rows = store(&original);
    let mut visited = vec![(8, 9)];
    retain_trace(&mut rows, &mut visited, 11);
    assert_eq!(visited, [(8, 9), (1, 10), (7, 11), (2, 12), (4, 20)]);
    assert_eq!(rows.entries(), [(2, 12), (4, 20)]);
}

#[test]
fn trace_all_and_none_keep_the_original_decision() {
    let original = [(0, 0), (u64::MAX, u64::MAX)];
    for floor in [0, u64::MAX] {
        let mut rows = store(&original);
        let mut visited = Vec::new();
        retain_trace(&mut rows, &mut visited, floor);
        assert_eq!(visited, original);
        let expected: Vec<_> = original.into_iter().filter(|row| row.1 > floor).collect();
        assert_eq!(rows.entries(), expected);
    }
}

#[test]
fn repeated_helper_inputs_keep_full_values_and_do_not_deduplicate() {
    let mut visited = vec![(7, 8)];
    let rows = [(7, 11), (7, 12), (7, 11)];
    let decisions: Vec<_> = rows
        .iter()
        .map(|(key, value)| trace_step(&mut visited, key, value, 11))
        .collect();
    assert_eq!(decisions, [false, true, false]);
    assert_eq!(visited, [(7, 8), (7, 11), (7, 12), (7, 11)]);
}

#[test]
fn split_toggle_matches_original_inline_body() {
    for initial in [false, true] {
        for len in 0..=4 {
            let source = [(1, 10), (7, 11), (2, 12), (4, 20)];
            let mut original = store(&source[..len]);
            let mut candidate = original.clone();
            let mut before = initial;
            let mut after = initial;
            original.retain(|_, _| {
                before = !before;
                before
            });
            retain_toggle(&mut candidate, &mut after);
            assert_eq!(candidate, original);
            assert_eq!(after, before);
        }
    }
}

#[test]
fn split_trace_matches_original_inline_body() {
    for floor in [0, 11, u64::MAX] {
        let source = [(0, 0), (7, 11), (2, 12), (u64::MAX, u64::MAX)];
        let mut original = store(&source);
        let mut candidate = original.clone();
        let mut before = vec![(8, 9)];
        let mut after = before.clone();
        original.retain(|key, value| {
            before.push((*key, *value));
            *value > floor
        });
        retain_trace(&mut candidate, &mut after, floor);
        assert_eq!(candidate, original);
        assert_eq!(after, before);
    }
}

#[test]
fn panic_preserves_already_visible_external_trace_prefix() {
    let mut rows = store(&[(1, 10), (2, 20), (3, 30)]);
    let mut visited = vec![(8, 9)];
    let result = catch_unwind(AssertUnwindSafe(|| {
        rows.retain(|key, value| {
            let keep = trace_step(&mut visited, key, value, 11);
            assert!(*key != 2, "controlled interruption after the second helper");
            keep
        });
    }));
    assert!(result.is_err());
    assert_eq!(visited, [(8, 9), (1, 10), (2, 20)]);
    // No claim about the Store's survivor sequence after unwinding.
}

#[test]
fn panic_does_not_roll_back_an_external_toggle() {
    let mut rows = store(&[(1, 10), (2, 20)]);
    let mut state = false;
    let result = catch_unwind(AssertUnwindSafe(|| {
        rows.retain(|key, value| {
            let keep = toggle_step(&mut state, key, value);
            assert!(*key != 1, "controlled interruption after the first helper");
            keep
        });
    }));
    assert!(result.is_err());
    assert!(
        state,
        "completed mutation remains visible across this native panic"
    );
}
