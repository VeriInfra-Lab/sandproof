//! Bounded original Retire receipt, independent of the center's later ack.
//! It is written only alongside the actual native Retire and exact source
//! removal. A missing native row alone never establishes cleanup.
use super::{codec,model::{Inbox,Session},source_work};
use crate::{InternalLocalBinding,JournalError};
use rusqlite::Connection;
use sandbox_controller_core::{self as core,maintenance::retirement::{RetireRequest,QuiescenceStatus}};
use sandproof_protocol::lifecycle::{ContentDigest,LifecycleOperation};
use serde::{Deserialize,Serialize};

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeRetirementReceipt {
    local:InternalLocalBinding,
    request_binding:ContentDigest,
    retirement:RetireRequest,
}

pub(super) fn validate_saved(session:&Session)->Result<(),JournalError>{
    let Some(saved)=&session.native_retirement else {return Ok(())};
    let last=session.entries.last().ok_or(JournalError::CorruptStore)?;
    let request=saved.retirement;
    let qc=request.quiescence;
    if !session.terminal || session.local!=Some(saved.local)
        || !matches!(last.intent.request.operation,LifecycleOperation::Destroy{..})
        || last.intent.accepted_version!=session.current_version
        || last.local_operation!=Some(request.operation)
        || saved.request_binding!=source_work::digest(&serde_json::to_vec(&last.intent)?)
        || request.sandbox!=saved.local.sandbox || request.owner!=saved.local.owner
        || qc.sandbox!=request.sandbox || qc.owner!=request.owner
        || qc.epoch!=request.epoch || qc.operation!=request.operation
        || qc.worker_drained!=QuiescenceStatus::Confirmed
        || qc.data_connections_drained!=QuiescenceStatus::Confirmed
        || qc.host_resources_reclaimed!=QuiescenceStatus::Confirmed
        || qc.external_pins_released!=QuiescenceStatus::Confirmed
        || serde_json::to_vec(saved)?.len()>8192
    {return Err(JournalError::CorruptStore)}
    Ok(())
}

pub(super) fn cleaned(session:&Session,world:&core::World)->Result<bool,JournalError>{
    validate_saved(session)?;
    let Some(saved)=&session.native_retirement else {return Ok(false)};
    if world.sandbox_record(saved.local.sandbox).is_some(){return Err(JournalError::CorruptStore)}
    Ok(true)
}

fn terminal_session(inbox:&Inbox,sandbox:core::SandboxId)->Option<&Session>{
    inbox.sessions.iter().find(|session|session.local.is_some_and(|local|local.sandbox==sandbox))
}

// Only the terminal original Retire path may discharge this metadata pin.
// General pinned/save logic remains unchanged for every other session.
pub(in crate::controller) fn ready(connection:&Connection,world:&core::World,sandbox:core::SandboxId)->Result<bool,JournalError>{
    let Some(inbox)=codec::load(connection)? else {return Ok(false)};
    let Some(session)=terminal_session(&inbox,sandbox) else {return Ok(false)};
    let local=session.local.ok_or(JournalError::CorruptStore)?;
    let last=session.entries.last().ok_or(JournalError::CorruptStore)?;
    let Some(row)=world.sandbox_record(sandbox) else {return Ok(false)};
    if !session.terminal || session.native_retirement.is_some()
        || !matches!(last.intent.request.operation,LifecycleOperation::Destroy{..})
        || row.owner!=local.owner
        || row.current_operation.is_none_or(|op|Some(op.id)!=last.local_operation)
        || core::maintenance::retirement::validate_terminal_cleanup(world,sandbox).is_err()
    {return Ok(false)}
    match source_work::validate_terminal_sources(connection,world,sandbox){
        Ok(_) => Ok(true),
        Err(JournalError::UnresolvedAction)=>Ok(false),
        Err(error)=>Err(error),
    }
}

// Called only after the SAME transaction applied this exact native request,
// before original SQL/source details are removed. No external caller flag.
pub(in crate::controller) fn record(connection:&Connection,before:&core::World,after:&core::World,request:RetireRequest)->Result<(),JournalError>{
    let Some(mut inbox)=codec::load(connection)? else {return Ok(())};
    let Some(index)=inbox.sessions.iter().position(|session|session.local.is_some_and(|local|local.sandbox==request.sandbox)) else {return Ok(())};
    if !ready(connection,before,request.sandbox)? || after.sandbox_record(request.sandbox).is_some(){return Err(JournalError::UnresolvedAction)}
    let session=&mut inbox.sessions[index];
    let local=session.local.ok_or(JournalError::CorruptStore)?;
    let original=before.sandbox_record(request.sandbox).ok_or(JournalError::CorruptStore)?;
    if request.owner!=original.owner || request.epoch!=original.epoch
        || request.quiescence.workspace!=original.workspace
        || original.current_operation.is_none_or(|op|op.id!=request.operation)
    {return Err(JournalError::Conflict)}
    session.native_retirement=Some(NativeRetirementReceipt{
        local,request_binding:source_work::digest(&serde_json::to_vec(&session.entries.last().ok_or(JournalError::CorruptStore)?.intent)?),retirement:request,
    });
    validate_saved(session)?;
    codec::save(connection,&inbox)
}
