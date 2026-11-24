import SaveLoadOriginal

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadSpec

open SaveLoadSql

structure CurrentPrefix (ports : SaveLoadSql.Ports) (db : Db) where
  presence : Bool
  version : queryVersion db = .ok (.Ok currentVersion)
  existsReply : queryExists db = .ok (.Ok presence)

structure FormatPrefix (ports : SaveLoadSql.Ports) (db : Db)
    extends CurrentPrefix ports db where
  schemaSource : schema ports.text db sourceName sourceSchema = .ok (.Ok ())
  schemaWork : schema ports.text db workName workSchema = .ok (.Ok ())

macro "reduce_current_prefix " stage:term : tactic =>
  `(tactic|
    (have metadataVersion := ($stage).version
     have metadataExists := ($stage).existsReply
     simp only [queryVersion, queryExists, queryZero] at metadataVersion metadataExists
     unfold SaveLoadOriginal.load SaveLoadOriginal.controller.lifecycle_inbox.source_work.codec.load
     simp only [metadataVersion, metadataExists, bind_tc_ok,
       core.result.Result.Insts.CoreOpsTry.branch, bne_self_eq_false, Bool.false_eq_true, if_false]))

macro "reduce_format_prefix " stage:term : tactic =>
  `(tactic|
    (reduce_current_prefix ($stage).toCurrentPrefix
     have sourceDdl := ($stage).schemaSource
     have workDdl := ($stage).schemaWork
     simp only [sourceName, workName] at sourceDdl workDdl
     simp only [sourceDdl, workDdl, bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]))

def Contract (loader : SaveLoadSql.Ports → Db → Result (Reply (Option EncoderSources))) : Prop :=
  ∀ (ports : SaveLoadSql.Ports) (db : Db) (_ : FormatPrefix ports db) (count : I64) (failure : SqlError),
    queryCount db = .ok (.Ok count) → queryNumbers db = .ok (.Err failure) →
    loader ports db = .ok (.Err (.Sqlite failure))

end SandboxModel.Stage1Protocol.SaveLoadSpec
