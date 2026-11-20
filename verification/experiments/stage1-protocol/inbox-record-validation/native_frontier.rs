    #[test]
    fn capture_frontier_checks_the_whole_value_and_actual_save() {
        for (capture, accepted) in [(6, false), (7, false), (8, true), (10, true)] {
            let mut inbox = resolved_unload();
            let connection = rusqlite::Connection::open_in_memory().unwrap();
            connection.execute_batch(
                "CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY, state_json BLOB, sha256 BLOB);\
                 INSERT INTO controller_lifecycle_inbox VALUES(1,X'00',X'00');",
            ).unwrap();
            codec::save(&connection, &inbox).unwrap();
            let row = || -> (Vec<u8>, Vec<u8>) {
                connection
                    .query_row(
                        "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap()
            };
            let before = row();
            let intent = unload(&inbox, 3, 3, capture);
            assert_eq!(inbox.record(&intent).unwrap(), (0, 2));
            check(&inbox, accepted);
            if accepted {
                codec::save(&connection, &inbox).unwrap();
                assert_ne!(row(), before);
            } else {
                assert!(matches!(
                    codec::save(&connection, &inbox),
                    Err(JournalError::CorruptStore)
                ));
                assert_eq!(row(), before);
            }
        }
    }
