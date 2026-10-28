// Candidate-only probes for the actual generic save body. No callback is assumed
// to implement correct JSON, SHA, authorization, rollback or durable commit.
#[cfg(test)]
mod save_effects_obligations {
    use super::*;
    use crate::controller::lifecycle_inbox::{
        codec::{InboxSaveEffects, save_using},
        model::StandardTerminalEncoding,
        native_retirement::StandardRetirementEncoding,
        renewal_confirmation::StandardRenewalEncoding,
        unload_resolution::StandardUnloadEncoding,
    };

    const SQL: &str = "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1";

    #[derive(Debug, PartialEq)]
    enum Event {
        Encode(Box<Inbox>),
        Digest(Vec<u8>),
        Update(String, Vec<u8>, Vec<u8>),
    }

    struct Effects {
        events: Vec<Event>,
        bytes: Vec<u8>,
        encoding_error: bool,
        update_error: bool,
        changed: usize,
    }

    impl Effects {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                events: vec![],
                bytes,
                encoding_error: false,
                update_error: false,
                changed: 1,
            }
        }
    }

    impl InboxSaveEffects for Effects {
        fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
            assert!(self.events.is_empty());
            self.events.push(Event::Encode(Box::new(inbox.clone())));
            if self.encoding_error {
                Err(JournalError::CorruptStore)
            } else {
                Ok(self.bytes.clone())
            }
        }

        fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
            assert_eq!(self.events.len(), 1);
            self.events.push(Event::Digest(bytes.to_vec()));
            [0xa5; 32]
        }

        fn update(
            &mut self,
            statement: &str,
            bytes: &[u8],
            digest: &[u8],
        ) -> Result<usize, JournalError> {
            assert_eq!(self.events.len(), 2);
            self.events.push(Event::Update(
                statement.to_owned(),
                bytes.to_vec(),
                digest.to_vec(),
            ));
            if self.update_error {
                Err(JournalError::UnsupportedStore)
            } else {
                Ok(self.changed)
            }
        }
    }

    fn save(inbox: &Inbox, effects: &mut Effects) -> Result<(), JournalError> {
        save_using::<
            StandardRetirementEncoding,
            StandardTerminalEncoding,
            StandardRenewalEncoding,
            StandardUnloadEncoding,
            crate::capture::StandardGraphicBytes,
            Effects,
        >(inbox, effects)
    }

    fn full_trace(inbox: &Inbox, bytes: &[u8]) -> Vec<Event> {
        vec![
            Event::Encode(Box::new(inbox.clone())),
            Event::Digest(bytes.to_vec()),
            Event::Update(SQL.to_owned(), bytes.to_vec(), vec![0xa5; 32]),
        ]
    }

    #[test]
    fn saved_bytes_digest_input_sql_and_effect_order_are_complete() {
        let inbox = attached();
        let before = inbox.clone();
        let bytes = vec![0, 0xff, 3, 4];
        let mut effects = Effects::new(bytes.clone());
        save(&inbox, &mut effects).unwrap();
        assert_eq!(effects.events, full_trace(&inbox, &bytes));
        assert_eq!(inbox, before);
    }

    #[test]
    fn failed_validation_skips_all_save_effects() {
        let mut inbox = recorded();
        inbox.sessions[0].current_version = 2;
        check(&inbox, false);
        let mut effects = Effects::new(vec![1, 2, 3]);
        assert!(matches!(
            save(&inbox, &mut effects),
            Err(JournalError::CorruptStore)
        ));
        assert!(effects.events.is_empty());
    }

    #[test]
    fn encode_error_preserves_its_effects_and_skips_digest_and_update() {
        let inbox = empty();
        let mut effects = Effects::new(vec![1]);
        effects.encoding_error = true;
        assert!(matches!(
            save(&inbox, &mut effects),
            Err(JournalError::CorruptStore)
        ));
        assert_eq!(effects.events, vec![Event::Encode(Box::new(inbox))]);
    }

    #[test]
    fn exact_budget_passes_and_one_extra_byte_stops_before_digest() {
        let inbox = empty();
        let limit = usize::try_from(inbox.config.required_bytes()).unwrap();
        let mut exact = Effects::new(vec![0x31; limit]);
        save(&inbox, &mut exact).unwrap();
        assert_eq!(exact.events, full_trace(&inbox, &exact.bytes));
        let mut extra = Effects::new(vec![0x32; limit + 1]);
        assert!(matches!(
            save(&inbox, &mut extra),
            Err(JournalError::MaintenanceCapacity)
        ));
        assert_eq!(extra.events, vec![Event::Encode(Box::new(inbox))]);
    }

    #[test]
    fn zero_or_two_rows_are_errors_after_the_update_effect() {
        let inbox = empty();
        for count in [0, 2] {
            let mut effects = Effects::new(vec![0x41, 0x42]);
            effects.changed = count;
            assert!(matches!(
                save(&inbox, &mut effects),
                Err(JournalError::CorruptStore)
            ));
            assert_eq!(effects.events, full_trace(&inbox, &effects.bytes));
        }
    }

    #[test]
    fn update_error_preserves_prior_effects_and_the_original_error() {
        let inbox = empty();
        let mut effects = Effects::new(vec![0x51]);
        effects.update_error = true;
        assert!(matches!(
            save(&inbox, &mut effects),
            Err(JournalError::UnsupportedStore)
        ));
        assert_eq!(effects.events, full_trace(&inbox, &effects.bytes));
    }

    #[test]
    fn generic_save_does_not_assume_encoded_bytes_are_valid_json() {
        let inbox = empty();
        let mut effects = Effects::new(vec![]);
        save(&inbox, &mut effects).unwrap();
        assert_eq!(effects.events, full_trace(&inbox, &[]));
    }
}
