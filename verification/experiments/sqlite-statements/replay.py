"""Reproduce only this formal module with original fixed tools, not native CI."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_sqlite_statements import check_tools, check_libraries, check_pair, check_extraction, read_json, sha
from prepare import prepare_archive
from translate import extract
from proof import run as prove


def run(root, archive, output, tool_root):
    if output.exists() or output.is_symlink():
        raise ValueError('new isolated evidence directory required')
    if not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('use the coordinated stable formal compiler target')
    check_tools(root, tool_root)
    check_libraries(root, tool_root)
    output.mkdir(parents=True,mode=0o700)
    start=time.monotonic()
    record=dict(kind='sqlite-statements-formal-module-not-full-suite',source_archive_sha256=sha(archive))
    source=output/'correct-source'
    record['materialization']=prepare_archive(archive,source)
    positive=extract(source,output/'correct-extraction',tool_root,'correct')
    if not positive['success']:
        raise ValueError('correct original-tool extraction failed')
    command=[sys.executable,str(ROOT/'verification/experiments/sqlite-statements/negative.py'),
        '--source-archive',str(archive),'--source',str(output/'wrong-source'),
        '--output',str(output/'wrong-extraction'),'--tool-root',str(tool_root)]
    with (output/'negative.log').open('xb') as log:
        if subprocess.run(command,cwd=root,stdout=log,stderr=subprocess.STDOUT,timeout=600).returncode:
            raise ValueError('wrong Rust did not compile and extract; this is not a semantic rejection')
    correct=check_extraction(root,output/'correct-extraction','correct')
    wrong=check_extraction(root,output/'wrong-extraction','accept-zero-changes')
    check_pair(root,correct,wrong)
    record['proof']=prove(root,output/'formal')
    record.update(success=True,seconds=time.monotonic()-start)
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--tool-root',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(run(ROOT,args.source_archive.resolve(strict=True),args.output.absolute(),args.tool_root.resolve(strict=True)),indent=2))
