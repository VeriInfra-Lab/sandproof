"""Fresh actual decoder extraction and finite assembly source admission."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import traceback
import support
from check import check


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='decode-assembly-source-admission', complete=False,
        a6_instance_adopted=False, library_refinement_proved=False,
        ning_acceptance=False, shared_acceptance=False, roots={})
    try:
        schema = support.schema.inventory(support.ROOT)
        frozen = support.bytes_module.inventory(support.ROOT)
        inputs = support.base.snapshot(support.ROOT)
        record.update(inputs=inputs, library_inputs=support.libraries(),
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=support.ROOT, text=True).strip())
        project = output / 'source'
        support.copy_source(project, inputs)
        for root in ('accepted_decode', 'recorded_decode'):
            raw, evidence = support.capture(project, root, output)
            if evidence['canonical_llbc_sha256'] != frozen['byte_roots'][root]['llbc_canonical_sha256']:
                raise ValueError('original complete decoder changed: ' + root)
            data = check(raw, schema)
            report = output / (root + '-assembly.json')
            report.write_text(json.dumps(data, indent=2)+'\n')
            record['roots'][root] = dict(**evidence, report_sha256=support.base.sha(report),
                assembly_checks=len(data['checked']), byte_key_matching_checked=data['byte_key_matching_checked'],
                expected_session_dispatch_checked=data['expected_session_dispatch_checked'],
                original_leaf_bodies=len(data['leaf_forwarding']['original_bodies']))
        for name, wanted in inputs.items():
            if support.base.sha(project / name) != wanted:
                raise ValueError('source copy changed during admission')
        if inputs != support.base.snapshot(support.ROOT) or record['library_inputs'] != support.libraries():
            raise ValueError('source/library inputs changed during admission')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
