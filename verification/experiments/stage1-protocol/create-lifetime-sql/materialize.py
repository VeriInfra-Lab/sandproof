"""Reuse exact shared SQL/string definitions without old business registrations."""
from pathlib import Path
import hashlib

SQL = 'verification/aeneas/sqlite-statements/SandproofSqliteStatements/FunsExternal.lean'
FOUNDATION = 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'
LITERAL = 'SandboxModel/Foundations/ErrorMessages.lean'


def fragments(root):
    def between(text, start, end):
        begin = text.index(start)
        return text[begin:text.index(end, begin)].rstrip()+'\n'
    sql, standard, literal = [(root/name).read_text() for name in [SQL, FOUNDATION, LITERAL]]
    return {
        'sql_to_str': between(sql, 'def toStr ', '\ndef I64.'),
        'sql_string_deref': between(sql,
            'def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref ', '\ndef rusqlite.transaction'),
        'sql_bind_values': between(sql, 'def bindValues ', '\n-- The double underscore'),
        'sql_execute': between(sql, 'def rusqlite.Connection.execute ', '\nend SandproofSqliteStatementsRust'),
        'string_empty': between(standard, '@[simp, rust_fun "core::str::{str}::is_empty"]',
            '\n@[simp, rust_fun "core::str::{str}::bytes"]'),
        'standard_to_str': between(standard, 'def toStr ', '\n@[simp, rust_fun'),
        'literal_bytes_math': between(literal, 'theorem literal_string_bytes_refines ',
            '\ntheorem default_string_write_refines'),
    }


def hashes(root):
    return {name: hashlib.sha256(value.encode()).hexdigest() for name, value in fragments(root).items()}


def install(root, project, destination, expected):
    pieces = fragments(root)
    if hashes(root) != expected:
        raise ValueError('reviewed common definition changed')
    files = {
        'SqliteCommonBinding.lean': '''import SandproofLifecycleSql.Types
import SandproofFoundations.IdentifierLibrary

/-! Exact shared definitions, one instance per independent view. The old
generated business Types also register conflicting unqualified discriminants.
Only the complete Params trait aliases the new transparent generated carrier.
No five-tuple binder is used as a two-array model. -/
open Aeneas Aeneas.Std
namespace SandproofSqliteStatementsRust
abbrev rusqlite.params.Params := SandproofLifecycleSqlRust.rusqlite.params.Params

'''+ '\n'.join(pieces[n] for n in ['sql_to_str', 'sql_string_deref', 'sql_bind_values', 'sql_execute'])+
            '\nend SandproofSqliteStatementsRust\n',
        'FiniteStringBinding.lean': '''import SandproofFoundations.TypesExternal

open Aeneas Aeneas.Std
namespace SandproofFoundationsRust
'''+pieces['string_empty']+'end SandproofFoundationsRust\n',
        'LiteralBytesBinding.lean': '''import SandproofLifecycleSql.SqliteCommonBinding
import SandboxModel.Foundations.ByteArray

open Aeneas Aeneas.Std
namespace SandproofFoundationsRust
'''+pieces['standard_to_str']+'''end SandproofFoundationsRust
namespace SandboxModel.Foundations
open SandproofFoundationsRust.IdentifierLibrary
'''+pieces['literal_bytes_math']+'end SandboxModel.Foundations\n',
    }
    installed = {}
    for leaf, text in files.items():
        target = project/destination/leaf
        target.write_text(text)
        installed[str(destination/leaf)] = hashlib.sha256(target.read_bytes()).hexdigest()
    return installed
