"""Admit the complete current JournalError carrier, without erasing variants."""

VARIANTS = (
    ('Io', 1), ('Sqlite', 1), ('CommitUncertain', 1), ('Json', 1),
    ('Contract', 1), ('Conflict', 0), ('IdentifierExhausted', 0),
    ('MaintenanceCapacity', 0), ('CaptureMaterialQuotaExceeded', 0),
    ('RevisionConflict', 0), ('ReceiptBeforeDispatch', 0),
    ('ConflictingReceipt', 0), ('UnsupportedStore', 0), ('CorruptStore', 0),
    ('CredentialIndexPoisoned', 0), ('InjectedCrash', 1),
    ('ControllerRejected', 1), ('ControllerBusy', 0), ('UnsupportedInput', 0),
    ('UnresolvedAction', 0), ('OperationFailed', 0), ('InvalidHostEvidence', 0),
    ('ObsoleteReceipt', 0),
)


def admit(raw, *, required=True):
    name = [{'Ident': [part, 0]} for part in
            ('sandproof_controller_service', 'JournalError')]
    declarations = [d for d in raw['translated']['type_decls']
                    if d and d['item_meta']['name'] == name]
    if not declarations and not required:
        return None
    if len(declarations) != 1:
        raise ValueError('missing or duplicated complete JournalError carrier')
    declaration = declarations[0]
    if declaration['item_meta']['opacity'] != 'Transparent':
        raise ValueError('opaque JournalError carrier is not admitted')
    variants = declaration['kind'].get('Enum')
    if variants is None or tuple((v['name'], len(v['fields'])) for v in variants) != VARIANTS:
        raise ValueError('JournalError variant order or payload differs')
    if [v['id'] for v in variants] != list(range(len(VARIANTS))):
        raise ValueError('JournalError variant identities differ')
    # Keep the full declaration: payload types, attributes, discriminants and
    # layout remain available to the source/alias comparison and evidence.
    return declaration


class CarrierAdmission:
    """Regressions run on each component's actual extracted type declaration."""

    def test_current_complete_error_carrier_is_admitted(self):
        admit(self.raw)

    def test_removed_quota_variant_is_rejected(self):
        from copy import deepcopy
        changed = deepcopy(self.raw)
        admit(changed)['kind']['Enum'].pop(8)
        with self.assertRaisesRegex(ValueError, 'variant order or payload'):
            admit(changed)

    def test_changed_error_payload_is_rejected(self):
        from copy import deepcopy
        changed = deepcopy(self.raw)
        admit(changed)['kind']['Enum'][0]['fields'].clear()
        with self.assertRaisesRegex(ValueError, 'variant order or payload'):
            admit(changed)

    def test_opaque_error_carrier_is_rejected(self):
        from copy import deepcopy
        changed = deepcopy(self.raw)
        admit(changed)['item_meta']['opacity'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'opaque JournalError'):
            admit(changed)
