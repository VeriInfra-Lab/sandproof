"""Make an isolated candidate by factoring the original policy, not copying it.

This is a source transformation for a reviewed candidate, not an equivalence
proof. The production tree is never changed. The generated manifest preserves
the exact old and new fragments for the implementation-connection review.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import tarfile
from pathlib import Path, PurePosixPath

BASELINE = 'e576a8475043c2d21b68231b7f6b1ead09be1952'
ARCHIVE_SHA256 = 'a96634e94e39f0e3377fa422283f018a4eb84f983a753eaacee2005b98d91a83'


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def function_fragment(text: str, marker: str) -> tuple[int, int, str]:
    """Locate these frozen functions; their strings have no curly braces."""
    begin = text.index(marker)
    opening = text.index('{', begin)
    depth = 1
    end = opening + 1
    while depth:
        depth += (text[end] == '{') - (text[end] == '}')
        end += 1
    return begin, end, text[opening + 1:end - 1]


BUFFER = '''
/// Fixed library operations only; the bounded policy remains in write_bounded.
/// Actual use is Vec<u8>; arbitrary resource-owning implementations are excluded.
pub trait BoundedBuffer: Sized {
    type ReserveError;
    fn new_buffer() -> Self;
    fn len(&self) -> usize;
    fn capacity(&self) -> usize;
    fn try_reserve_exact(&mut self, additional: usize) -> Result<(), Self::ReserveError>;
    fn extend_from_slice(&mut self, bytes: &[u8]);
    fn into_bytes(self) -> Vec<u8>;
    fn capacity_error() -> io::Error;
    fn allocation_error() -> io::Error;
}

impl BoundedBuffer for Vec<u8> {
    type ReserveError = std::collections::TryReserveError;
    fn new_buffer() -> Self { Vec::new() }
    fn len(&self) -> usize { Vec::len(self) }
    fn capacity(&self) -> usize { Vec::capacity(self) }
    fn try_reserve_exact(&mut self, additional: usize) -> Result<(), Self::ReserveError> {
        Vec::try_reserve_exact(self, additional)
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) { Vec::extend_from_slice(self, bytes); }
    fn into_bytes(self) -> Vec<u8> { self }
    fn capacity_error() -> io::Error { io::Error::other("bounded evidence exceeded") }
    fn allocation_error() -> io::Error { io::Error::other("bounded evidence allocation failed") }
}

pub struct BoundedWriter<B: BoundedBuffer> {
    pub bytes: B,
    pub limit: usize,
    pub exceeded: bool,
}

impl<B: BoundedBuffer> Write for BoundedWriter<B> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        write_bounded(self, bytes)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
'''


def prepare(archive: Path, destination: Path, crate_archive: Path | None, variant: str = "correct") -> None:
    assert digest(archive.read_bytes()) == ARCHIVE_SHA256, 'Unexpected production archive'
    assert not destination.exists(), 'Refuse to reuse a candidate work directory'
    destination.mkdir(parents=True, mode=0o700)
    for source in [archive] + ([crate_archive] if crate_archive else []):
        assert source is not None
        with tarfile.open(source) as tf:
            for member in tf.getmembers():
                name = PurePosixPath(member.name)
                assert not name.is_absolute() and '..' not in name.parts
                assert member.isfile() or member.isdir(), 'No links or devices in this candidate'
                if source == crate_archive:
                    assert (member.name == 'verification/extract-bounded-writer' or member.name.startswith('verification/extract-bounded-writer/'))
            tf.extractall(destination)

    assert variant in ('correct', 'no-limit'), 'Unknown variant'
    evidence = destination / 'controller-service/src/template_registry/evidence.rs'
    old_evidence = evidence.read_text()
    begin, end, original_body = function_fragment(old_evidence, 'pub fn encode_bounded(')
    _, _, write_body = function_fragment(original_body, 'fn write(&mut self, bytes: &[u8])')
    assert write_body.count('io::Error::other("bounded evidence exceeded")') == 2
    assert write_body.count('io::Error::other("bounded evidence allocation failed")') == 1
    policy_body = write_body.replace('self.', 'writer.')
    policy_body = policy_body.replace('io::Error::other("bounded evidence exceeded")', 'B::capacity_error()')
    policy_body = policy_body.replace('io::Error::other("bounded evidence allocation failed")', 'B::allocation_error()')
    assert policy_body.count('if length > writer.limit {') == 1
    if variant == 'no-limit':
        policy_body = policy_body.replace('if length > writer.limit {', 'if false {')
    finish_body = original_body[original_body.index('    if writer.exceeded {'):]
    assert finish_body.count('Ok(writer.bytes)') == 1
    finish_body = finish_body.replace('Ok(writer.bytes)', 'Ok(writer.bytes.into_bytes())')
    replacement = BUFFER + '''
/// The original write policy, with fixed library operations passed explicitly.
pub fn write_bounded<B: BoundedBuffer>(writer: &mut BoundedWriter<B>, bytes: &[u8]) -> io::Result<usize> {
''' + policy_body + '''
}

/// Preserve the original exceeded-before-serialization-result priority.
pub fn finish_bounded<B: BoundedBuffer>(writer: BoundedWriter<B>, result: Result<(), serde_json::Error>) -> Result<Vec<u8>, JournalError> {
''' + finish_body + '''
}

/// Only the standard serialization call and original writer lifetime surround the policy.
pub fn encode_bounded_with_buffer<B: BoundedBuffer>(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, JournalError> {
    let mut writer = BoundedWriter { bytes: B::new_buffer(), limit, exceeded: false };
    let result = serde_json::to_writer(&mut writer, value);
    finish_bounded(writer, result)
}

pub fn encode_bounded(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, JournalError> {
    encode_bounded_with_buffer::<Vec<u8>>(value, limit)
}
'''
    evidence.write_text(old_evidence[:begin] + replacement + old_evidence[end:])

    server = destination / 'sandbox-internal/src/server.rs'
    old_server = server.read_text()
    start, finish, original_response = function_fragment(old_server, 'fn json_response(')
    call = 'sandproof_controller_service::template_registry::evidence::encode_bounded(answer, limit)'
    assert original_response.count(call) == 1
    result_body = original_response.replace(call, 'result')
    response = '''pub trait ProofJsonResponseBuilder {
    type Response;
    fn empty_error() -> Self::Response;
    fn json_ok(encoded: Vec<u8>) -> Self::Response;
}

pub struct ProofHttpJsonResponseBuilder;
impl ProofJsonResponseBuilder for ProofHttpJsonResponseBuilder {
    type Response = Response<Full<Bytes>>;
    fn empty_error() -> Self::Response {
        response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new())
    }
    fn json_ok(encoded: Vec<u8>) -> Self::Response {
        let mut answer = response(StatusCode::OK, Bytes::from(encoded));
        answer.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
        answer
    }
}

fn json_response(answer: &impl serde::Serialize, limit: usize) -> Response<Full<Bytes>> {
    proof_json_response_result::<ProofHttpJsonResponseBuilder>(sandproof_controller_service::template_registry::evidence::encode_bounded(answer, limit))
}

/// The original result-selection body, separated from the library call.
pub fn proof_json_response_result<P: ProofJsonResponseBuilder>(result: Result<Vec<u8>, sandproof_controller_service::JournalError>) -> P::Response {
    match result {
        Ok(encoded) => P::json_ok(encoded),
        Err(_error) => P::empty_error(),
    }
}
'''
    server.write_text(old_server[:start] + response + old_server[finish:])

    library = destination / 'sandbox-internal/src/lib.rs'
    old_library = library.read_text()
    library.write_text(old_library + '\n// Verification-only access to the factored policy.\npub use server::{ProofHttpJsonResponseBuilder, ProofJsonResponseBuilder, proof_json_response_result};\n')

    changes = {
        'kind': 'isolated-source-candidate-not-general-equivalence-proof',
        'baseline': BASELINE, 'archive_sha256': ARCHIVE_SHA256, 'variant': variant,
        'evidence_before_sha256': digest(old_evidence.encode()),
        'evidence_after_sha256': digest(evidence.read_bytes()),
        'server_before_sha256': digest(old_server.encode()),
        'server_after_sha256': digest(server.read_bytes()),
        'original_write_body': write_body, 'candidate_write_body': policy_body,
        'original_finish_body': original_body[original_body.index('    if writer.exceeded {'):],
        'candidate_finish_body': finish_body,
        'original_response_body': original_response, 'candidate_response_body': result_body,
        'actual_response_candidate': response,
        'internal_lib_before_sha256': digest(old_library.encode()),
        'internal_lib_after_sha256': digest(library.read_bytes()),
        'unmodified_remainder': True,
        'library_operations': ['Vec::new/len/capacity/try_reserve_exact/extend_from_slice',
                               'identity into_bytes', 'two original io::Error::other literal constructors'],
        'connection_status': 'The module proves candidate semantics separately; actual Vec/library binding and original-to-candidate source factorization remain explicit conditions.',
    }
    output = destination / 'artifacts/bounded-writer-candidate'
    output.mkdir(parents=True)
    (output / 'source-transform.json').write_text(json.dumps(changes, indent=2) + '\n')
    print(json.dumps({'source': str(destination), 'changes': str(output / 'source-transform.json')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source-archive', type=Path, required=True)
    parser.add_argument('--crate-archive', type=Path)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct', 'no-limit'], default='correct')
    args = parser.parse_args()
    prepare(args.source_archive, args.destination, args.crate_archive, args.variant)
