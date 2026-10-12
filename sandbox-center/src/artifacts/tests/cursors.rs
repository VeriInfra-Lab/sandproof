use super::*;

#[test]
fn artifacts_rotate_independently_without_growing_per_attempt() {
    let nodes = std::num::NonZeroUsize::new(3).unwrap();
    let first = "first-r1".try_into().unwrap();
    let second = "second-r1".try_into().unwrap();
    let mut cursors = Vec::new();
    for expected in [0, 1, 2, 0, 1] {
        assert_eq!(rotate_start(&mut cursors, &first, [1; 32], nodes), expected);
        assert_eq!(
            rotate_start(&mut cursors, &second, [2; 32], nodes),
            expected
        );
        assert_eq!(cursors.len(), 2);
    }
}

#[test]
fn cursor_is_bounded_for_a_single_host_and_after_node_count_changes() {
    let revision: ResourceId = "first-r1".try_into().unwrap();
    let mut cursors = vec![CacheStart {
        revision: revision.clone(),
        digest: [1; 32],
        next_host: usize::MAX,
    }];
    assert_eq!(
        rotate_start(
            &mut cursors,
            &revision,
            [1; 32],
            std::num::NonZeroUsize::new(1).unwrap()
        ),
        0
    );
    assert_eq!(cursors[0].next_host, 0);
    assert_eq!(
        rotate_start(
            &mut cursors,
            &revision,
            [1; 32],
            std::num::NonZeroUsize::new(3).unwrap()
        ),
        0
    );
    assert_eq!(cursors[0].next_host, 1);
}
