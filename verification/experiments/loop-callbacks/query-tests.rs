
#[cfg(test)]
mod loop_callbacks_query_contract {
    use super::Store;

    #[test]
    fn empty_and_absent_queries_preserve_none() {
        let empty = Store::<u64, u64>::default();
        assert_eq!(empty.get(&7), None);
        assert!(!empty.contains_key(&7));
        let mut rows = Store::default();
        rows.put(1_u64, 11_u64);
        assert_eq!(rows.get(&7), None);
        assert!(!rows.contains_key(&7));
    }

    #[test]
    fn duplicate_key_returns_original_first_matching_row() {
        let rows = Store {
            entries: vec![(7_u64, 11_u64), (7, 12), (2, 20)],
        };
        let returned = rows.get(&7).unwrap();
        assert_eq!(
            *returned, 11,
            "lookup must return original first matching row"
        );
        assert!(std::ptr::eq(returned, &raw const rows.entries[0].1));
        assert_eq!(rows.entries[1].1, 12);
    }

    #[test]
    fn borrowed_noncopy_value_is_the_original_value() {
        let rows = Store {
            entries: vec![(7_u64, String::from("original")), (7, String::from("later"))],
        };
        let value = rows.get(&7).unwrap();
        assert_eq!(value, "original");
        assert!(std::ptr::eq(value, &raw const rows.entries[0].1));
    }

    #[test]
    fn extreme_keys_and_values_keep_their_full_values() {
        let rows = Store {
            entries: vec![(0_u64, u64::MAX), (u64::MAX, 0_u64)],
        };
        assert_eq!(rows.get(&0), Some(&u64::MAX));
        assert_eq!(rows.get(&u64::MAX), Some(&0));
        assert!(rows.contains_key(&0));
        assert!(rows.contains_key(&u64::MAX));
    }

    #[test]
    fn contains_key_agrees_with_original_lookup_presence() {
        let rows = Store {
            entries: vec![(7_u64, 11_u64), (7, 12)],
        };
        assert_eq!(
            rows.contains_key(&7),
            rows.get(&7).is_some(),
            "contains_key must agree with original lookup presence"
        );
        assert_eq!(rows.contains_key(&1), rows.get(&1).is_some());
    }

    #[test]
    fn shared_queries_keep_all_rows_in_original_order() {
        let rows = Store {
            entries: vec![(3_u64, 1_u64), (7, 11), (7, 12)],
        };
        let original = rows.entries.clone();
        let _ = rows.get(&7);
        let _ = rows.get(&u64::MAX);
        let _ = rows.contains_key(&3);
        assert_eq!(rows.entries, original);
    }
}
