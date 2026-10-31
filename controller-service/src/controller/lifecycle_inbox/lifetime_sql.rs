//! Preserve the two complete lifetime UPDATE values at the original execute API.
//! The caller retains the connection, SQL and successful row-count policy.
use crate::JournalError;
use rusqlite::{
    Connection,
    types::{ToSqlOutput, ValueRef},
};

pub(super) fn execute(
    connection: &Connection,
    statement: &str,
    deadline: i64,
    sandbox: &str,
) -> Result<usize, JournalError> {
    Ok(connection.execute(
        statement,
        [
            ToSqlOutput::Borrowed(ValueRef::Integer(deadline)),
            ToSqlOutput::Borrowed(ValueRef::Text(sandbox.as_bytes())),
        ],
    )?)
}
