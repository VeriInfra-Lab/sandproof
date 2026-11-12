"""Bounded XFS project quota observations shared by installation and acceptance."""
import os
import selectors
import subprocess
import time


ENV={'PATH':'/usr/bin:/bin','LANG':'C'}


def project_hard_limit(text, project):
    # No -h/-H: xfs_quota report uses exact 1-KiB blocks (-b), or inode
    # counts (-i). Explicit numeric project (-n) avoids name-service aliases.
    matches=[]
    for line in text.splitlines():
        fields=line.split()
        if fields and fields[0] in (str(project),'#'+str(project)):
            if len(fields)<4 or any(not value.isdecimal() for value in fields[1:4]):
                raise ValueError('incomplete numeric original quota row')
            if int(fields[2])!=0:
                raise ValueError('expected fixed disabled soft limit')
            matches.append(int(fields[3]))
    if len(matches)!=1 or matches[0]<=0:
        raise ValueError('one exact nonzero project hard limit required')
    return matches[0]


def capture(tool,command,root):
    # Existing fixed tool only, no shell/user-supplied command program.
    process=subprocess.Popen([str(tool),'-x','-c',command,str(root)],stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=ENV)
    output=bytearray()
    deadline=time.monotonic()+10
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout,selectors.EVENT_READ)
            selector.register(process.stderr,selectors.EVENT_READ)
            while selector.get_map():
                remaining=deadline-time.monotonic()
                if remaining<=0:
                    raise TimeoutError('original quota readback deadline')
                events=selector.select(remaining)
                if not events:
                    raise TimeoutError('original quota readback deadline')
                for key,_ in events:
                    data=os.read(key.fileobj.fileno(),min(4096,65537-len(output)))
                    if not data:
                        selector.unregister(key.fileobj)
                    elif key.fileobj is process.stderr:
                        raise ValueError('quota diagnostic output is not successful readback')
                    else:
                        output.extend(data)
                        if len(output)>65536:
                            raise ValueError('quota readback exceeds original output bound')
        if process.wait(timeout=max(.001,deadline-time.monotonic()))!=0:
            raise ValueError('quota readback command failed')
        return output.decode('ascii')
    finally:
        process.stdout.close();process.stderr.close()
        if process.poll() is None:
            process.kill();process.wait(timeout=3)
