import SourceSerdeDecodeView
import SourceSerdeSome

open Aeneas Aeneas.Std SandproofStage1RequestRust
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SourceSerdeDecodeProof
open SourceSerdeDecodeView
noncomputable section

theorem original_decode_forwards_full_input_and_optional_dictionary (ports : Ports) (bytes : Slice U8) :
    decode ports bytes = ports.fromSlice (optionDictionary ports) bytes := by rfl

theorem original_record_dictionary_forwards_exactly (ports : Ports) {D E : Type}
    (dictionary : serde_core.de.Deserializer D E) (input : D) :
    (bareDictionary ports).deserialize dictionary input = ports.record dictionary input := by rfl

theorem original_option_dictionary_calls_same_required_option_provider (ports : Ports) {D E : Type}
    (dictionary : serde_core.de.Deserializer D E) (input : D) :
    (optionDictionary ports).deserialize dictionary input =
      SourceSerdeRequiredView.required ports.option dictionary (bareDictionary ports) input := by rfl

/-- Instantiate the actual record loader with the real typed from_slice
    wrapper. The two precise library contracts supply the original roundtrip;
    the inner concrete record/field semantics remain explicitly named. -/
def actualSerde (json : SaveRoundTripShared.SerdePorts) (ports : Ports) : SaveRoundTripShared.SerdePorts :=
  { json with parse := decode ports }

theorem actual_typed_decode_uses_named_roundtrip_contract {W R : Type}
    (json : SaveRoundTripShared.SerdePorts) (ports : Ports)
    (writer : SourceSerdeSome.VecSymbols (actualSerde json ports) W)
    (reader : SourceSerdeSome.SliceSymbols (actualSerde json ports) R) :
    SaveRoundTripShared.SerdeSomeContract (actualSerde json ports) :=
  SourceSerdeSome.actual_named_library_symbols_provide_roundtrip_contract _ writer reader


/-- The actual typed from_slice wrapper, the original complete encoder, and
    the original ordered reader share the precise named library providers. -/
theorem actual_typed_decoder_roundtrips_complete_records {Keys W R : Type}
    (encoder : SandproofSaveRetentionRows.EncoderPorts Keys)
    (json : SaveRoundTripShared.SerdePorts) (ports : Ports)
    (writerSymbols : SourceSerdeSome.VecSymbols (actualSerde json ports) W)
    (readerSymbols : SourceSerdeSome.SliceSymbols (actualSerde json ports) R)
    (sources : SandproofSaveRetentionRows.EncoderSources) (keys : Keys)
    (rows : SandproofSaveRetentionRows.EncoderRows) (raw : SaveLoadCursorView.RawCursor)
    (encoded : SandproofSaveRetentionRows.encodeCandidate
      (SaveRoundTripShared.sharedEncoder encoder (actualSerde json ports)) sources = .ok (.Ok (keys, rows)))
    (physicalProjection : List.Forall₂ SaveRoundTripShared.WrittenProjection rows.val raw.val) :
    SaveLoadCollectorView.reader (SaveLoadCanonicalView.collector
      (SaveRoundTripShared.sharedCodec (actualSerde json ports))) SaveLoadCursorView.cursorDictionary
      (SaveLoadCursorView.mappedCursor raw) = .ok (.Ok sources.records) :=
  SaveRoundTripShared.original_encoder_and_reader_roundtrip_complete_records encoder (actualSerde json ports)
    (actual_typed_decode_uses_named_roundtrip_contract json ports writerSymbols readerSymbols)
    sources keys rows raw encoded physicalProjection

end
end SandboxModel.Stage1Protocol.SourceSerdeDecodeProof

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeDecodeProof.original_decode_forwards_full_input_and_optional_dictionary
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeDecodeProof.original_record_dictionary_forwards_exactly
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeDecodeProof.original_option_dictionary_calls_same_required_option_provider
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeDecodeProof.actual_typed_decode_uses_named_roundtrip_contract

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeDecodeProof.actual_typed_decoder_roundtrips_complete_records
