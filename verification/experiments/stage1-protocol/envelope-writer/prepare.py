"""Apply the reviewed original Writer factorization in a private source copy.

Source correspondence remains an explicit B4 condition. The writer policy is
extracted from the current production body and checked against the prior exact
fragments; it is never replaced by a proof-only policy implementation.
"""
from pathlib import Path
import argparse
import difflib
import hashlib
import importlib.util
import json


def sha(data):
    return hashlib.sha256(data).hexdigest()


def prepare(project, variant='correct'):
    if variant not in ('correct', 'swallow-error'):
        raise ValueError('unknown writer variant')
    path = project / 'verification/experiments/bounded-writer/prepare.py'
    spec = importlib.util.spec_from_file_location('original_writer_factorization', path)
    old = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(old)
    registered = json.loads((path.parent / 'candidate.json').read_text())
    evidence = project / 'controller-service/src/template_registry/evidence.rs'
    original = evidence.read_text()
    start, end, body = old.function_fragment(original, 'pub fn encode_bounded(')
    _, _, write = old.function_fragment(body, 'fn write(&mut self, bytes: &[u8])')
    finish = body[body.index('    if writer.exceeded {'):]
    if write != registered['original_write_body'] or finish != registered['original_finish_body']:
        raise ValueError('original writer or finish changed; review required')
    policy = write.replace('self.', 'writer.').replace(
        'io::Error::other("bounded evidence exceeded")', 'B::capacity_error()').replace(
        'io::Error::other("bounded evidence allocation failed")', 'B::allocation_error()')
    finish = finish.replace('Ok(writer.bytes)', 'Ok(writer.bytes.into_bytes())')
    if policy != registered['candidate_write_body'] or finish != registered['candidate_finish_body']:
        raise ValueError('original reviewed factorization differs')
    initialization = body[body.index('    let mut writer = Writer {'):body.index('    let result = serde_json::to_writer')]
    initialization = initialization.replace('= Writer {', '= BoundedWriter {').replace('Vec::new()', 'B::new_buffer()')
    ending = '    finish_bounded(writer, result)'
    if variant == 'swallow-error':
        ending = '    let _ = result;\n    finish_bounded(writer, Ok(()))'
    replacement = old.BUFFER + '''
/// Original checked write policy; unchanged from the reviewed factorization.
pub fn write_bounded<B: BoundedBuffer>(writer: &mut BoundedWriter<B>, bytes: &[u8]) -> io::Result<usize> {
''' + policy + '''
}

/// Original exceeded-before-serializer-result decision.
pub fn finish_bounded<B: BoundedBuffer>(writer: BoundedWriter<B>, result: Result<(), serde_json::Error>) -> Result<Vec<u8>, JournalError> {
''' + finish + '''
}

/// Exact standard serialization call, with its original mutable Writer and value.
pub trait BoundedSerialize<W, T> {
    fn serialize(writer: &mut W, value: &T) -> Result<(), serde_json::Error>;
}
pub struct StandardSerialize;
impl<B: BoundedBuffer, T: serde::Serialize> BoundedSerialize<BoundedWriter<B>, T> for StandardSerialize {
    fn serialize(writer: &mut BoundedWriter<B>, value: &T) -> Result<(), serde_json::Error> {
        serde_json::to_writer(writer, value)
    }
}

pub fn encode_bounded_using<B, T, S>(value: &T, limit: usize) -> Result<Vec<u8>, JournalError>
where B: BoundedBuffer, S: BoundedSerialize<BoundedWriter<B>, T>,
{
''' + initialization + '''    let result = S::serialize(&mut writer, value);
''' + ending + '''
}

pub fn encode_bounded_with_buffer<B: BoundedBuffer>(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, JournalError> {
    encode_bounded_using::<B, _, StandardSerialize>(value, limit)
}

pub fn encode_bounded(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, JournalError> {
    encode_bounded_with_buffer::<Vec<u8>>(value, limit)
}
'''
    changed = original[:start] + replacement + original[end:]
    evidence.write_text(changed)
    server = project / 'sandbox-internal/src/server.rs'
    server_before = server.read_text()
    a, b, response = old.function_fragment(server_before, 'fn json_response(')
    if response != registered['original_response_body']:
        raise ValueError('original response changed; review required')
    server_after = server_before[:a] + registered['actual_response_candidate'].rstrip() + server_before[b:]
    server.write_text(server_after)
    library = project / 'sandbox-internal/src/lib.rs'
    library_before = library.read_text()
    library_after = library_before + '\n// Private verification candidate only.\npub use server::{ProofHttpJsonResponseBuilder, ProofJsonResponseBuilder, proof_json_response_result};\n'
    library.write_text(library_after)
    out = project / 'artifacts/envelope-writer-source'
    out.mkdir(parents=True)
    records = []
    diffs = []
    for path, before, after in [(evidence, original, changed), (server, server_before, server_after),
                                (library, library_before, library_after)]:
        name = str(path.relative_to(project))
        records.append(dict(path=name, before_sha256=sha(before.encode()), after_sha256=sha(after.encode())))
        diffs.extend(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
            fromfile='a/' + name, tofile='b/' + name))
    (out / 'candidate.patch').write_text(''.join(diffs))
    record = dict(variant=variant, prior_baseline=registered['baseline'],
        original_write_unchanged=True, original_finish_unchanged=True, original_response_unchanged=True,
        original_encode_body=body, candidate_encode_initialization=initialization,
        condition='B4 exact factorization/callback binding; not general machine equivalence', files=records)
    (out / 'source-transform.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', required=True, type=Path)
    parser.add_argument('--variant', choices=['correct', 'swallow-error'], default='correct')
    args = parser.parse_args()
    print(json.dumps(prepare(args.project.absolute(), args.variant)))
