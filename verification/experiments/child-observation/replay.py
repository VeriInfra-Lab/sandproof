"""Regenerate all three roots with the unchanged pinned Charon and Aeneas."""
import argparse
import json
from pathlib import Path
import sys
import time
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from prepare import prepare
from translate import run as translate
from tools.check_child_observation import check_sources,check_tools,check_extraction

def run(root,archive,output,tools):
    check_sources(root);check_tools(root,tools)
    if output.exists():raise ValueError('fresh output required')
    output.mkdir(parents=True,mode=0o700);start=time.monotonic();record={'variants':{}}
    for variant in ['correct','caller','kill-success-release']:
        source=output/(variant+'-source');materialized=prepare(archive,source,'correct' if variant=='caller' else variant)
        target='crate::child_process::_::stop_and_reap' if variant=='caller' else 'crate::child_process::observation_core::stop'
        path=output/variant;translate(source,path,tools,target);check_extraction(root,path,variant)
        record['variants'][variant]=materialized
    record.update(success=True,seconds=time.monotonic()-start)
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n');return record

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=ROOT);p.add_argument('--archive',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--tool-root',type=Path,required=True)
    a=p.parse_args();print(json.dumps(run(a.root.resolve(strict=True),a.archive.resolve(strict=True),a.output.absolute(),a.tool_root.resolve(strict=True)),indent=2))
