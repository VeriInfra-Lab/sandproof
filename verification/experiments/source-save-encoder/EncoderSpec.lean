import EncoderPorts

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveEncoder

abbrev Row := SourceWorkRecord
abbrev Encoded := EncoderKey × Vec U8
abbrev Iter := core.slice.iter.Iter Row

def keyOf (record : Row) : EncoderKey := record.work.requirement.work_binding
def inputKeys (rows : List Row) : List EncoderKey := rows.map keyOf
def outputKeys (rows : List Encoded) : List EncoderKey := rows.map Prod.fst

/-- The exact successful operations on one returned carrier. A failure of an
operation is allowed, and only a successful, new insertion advances the loop. -/
structure KeySetSpec {Keys : Type} (ports : EncoderPorts Keys) (has : Keys → EncoderKey → Bool) : Prop where
  empty : ∀ keys, ports.empty = .ok keys → ∀ key, has keys key = false
  added : ∀ before key after, ports.insert before key = .ok (true, after) →
    has before key = false ∧ ∀ item, has after item = (has before item || decide (item = key))
  contains : ∀ keys key, ports.contains keys key = .ok (has keys key)

def KeySetFor {Keys : Type} (has : Keys → EncoderKey → Bool) (keys : Keys) (expected : List EncoderKey) : Prop :=
  ∀ key, has keys key = decide (key ∈ expected)

def EncodedRowOf {Keys : Type} (ports : EncoderPorts Keys) (record : Row) (row : Encoded) : Prop :=
  row.1 = keyOf record ∧ ports.recordJson record = .ok (.Ok row.2)

def remaining (iter : Iter) : List Row := iter.slice.val.drop iter.i

def CompleteEncoding {Keys : Type} (ports : EncoderPorts Keys) (has : Keys → EncoderKey → Bool)
    (sources : EncoderSources) (keys : Keys) (result : EncoderRows) : Prop :=
  List.Forall₂ (EncodedRowOf ports) sources.records.val result.val ∧
    outputKeys result.val = inputKeys sources.records.val ∧
    KeySetFor has keys (inputKeys sources.records.val)

end SandboxModel.Stage1Protocol.SaveEncoder
