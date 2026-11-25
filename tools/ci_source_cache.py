"""Publish and share a verified, bounded, read-only PR source disk."""
import argparse
from contextlib import contextmanager
from dataclasses import asdict, dataclass
import fcntl
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time


REPOSITORY = 'VeriInfra-Lab/sandproof'
URL = 'https://github.com/' + REPOSITORY + '.git'
SETTINGS = Path('/etc/sandproof-ci/source-cache.json')
CACHE = Path('/var/lib/sandproof-ci/source-cache')
MOUNT = '/opt/sandproof-ci/source'
GUEST_BIN = '/opt/sandproof-ci/source-bin'
LABEL = 'SANDCI-SOURCE'
MAX_PACK = 64 * 1024**2
MAX_DISK = 72 * 1024**2
MAX_ENTRIES = 12
MAX_CACHE = 512 * 1024**2
MAX_IDLE = 48 * 3600
FAILURE_BACKOFF = 60
SHA = re.compile(r'[0-9a-f]{40}')
DIGEST = re.compile(r'[0-9a-f]{64}')
REQUEST_PATH = re.compile(r'/v1/source/([1-9][0-9]{0,8})/([0-9a-f]{40})/([0-9a-f]{40})')


@dataclass(frozen=True)
class PullSource:
    number: int
    head: str
    base: str

    def __post_init__(self):
        if (type(self.number) is not int or not 1 <= self.number <= 999999999
                or not all(isinstance(v,str) and SHA.fullmatch(v) for v in (self.head,self.base))):
            raise ValueError('invalid PR source identity')

    @property
    def request_path(self):
        return f'/v1/source/{self.number}/{self.head}/{self.base}'


@dataclass(frozen=True)
class SourceDisk:
    image: Path
    source: PullSource
    merge: str
    tree: str


def from_run(run):
    """The trusted Actions inventory is a hint; the guest still checks GITHUB_SHA."""
    linked = run.get('pull_requests')
    if run.get('event') != 'pull_request' or not isinstance(linked,list) or len(linked)!=1:
        return None
    row = linked[0]
    try:
        source = PullSource(row['number'],row['head']['sha'],row['base']['sha'])
    except (KeyError,TypeError,ValueError):
        return None
    return source if run.get('head_sha') == source.head else None


def load_settings(path=SETTINGS):
    if not path.exists() and not path.is_symlink():
        return None
    attrs = path.lstat()
    if not stat.S_ISREG(attrs.st_mode) or attrs.st_uid != 0 or attrs.st_mode & 0o077:
        raise ValueError('source cache settings must be root-private')
    values=json.loads(path.read_text())
    if (not isinstance(values,dict) or set(values)!={'publisher_port','socks_port'}
            or any(type(p) is not int or not 1024<=p<=65535 for p in values.values())):
        raise ValueError('invalid source cache port settings')
    return values


def digest(path):
    result=hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024**2),b''):
            result.update(block)
    return result.hexdigest()


def git(repo,*args, **kwargs):
    env=dict(PATH='/usr/bin:/bin',GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1',GIT_TERMINAL_PROMPT='0')
    # The publisher's service has a fixed task budget even on a many-core host.
    cmd=['/usr/bin/prlimit',f'--fsize={MAX_DISK}','--','/usr/bin/git','-c','pack.threads=2','-C',str(repo),*args]
    return subprocess.run(cmd,env=env,check=True,timeout=45,**kwargs)


def verify_repo(repo,source,revision):
    if not isinstance(revision,str) or not SHA.fullmatch(revision):
        raise ValueError('invalid synthetic merge commit')
    header=git(repo,'cat-file','commit',revision,capture_output=True,text=True).stdout.split('\n\n',1)[0]
    parents=[row.split()[1] for row in header.splitlines() if row.startswith('parent ')]
    if parents!=[source.base,source.head]:
        raise ValueError('GitHub merge ref has different parents')
    tree=git(repo,'rev-parse',revision+'^{tree}',capture_output=True,text=True).stdout.strip()
    if not SHA.fullmatch(tree):
        raise ValueError('invalid source tree identity')
    return tree


def build_pack(root,source):
    repo=root/'publisher.git'
    repo.mkdir()
    git(repo,'init','--bare','--quiet')
    ref=f'+refs/pull/{source.number}/merge:refs/heads/pinned'
    git(repo,'-c','transfer.fsckObjects=true','fetch','--quiet','--depth=1','--no-tags',
        '--no-recurse-submodules',URL,ref,capture_output=True)
    revision=git(repo,'rev-parse','refs/heads/pinned',capture_output=True,text=True).stdout.strip()
    verify_repo(repo,source,revision)
    pack=root/'source.pack'
    with pack.open('xb') as stream:
        git(repo,'pack-objects','--stdout','--revs',input=(revision+'\n').encode(),stdout=stream)
    if not 0<pack.stat().st_size<=MAX_PACK:
        raise ValueError('PR source exceeds fixed download limit')
    return revision,pack


class Publisher(BaseHTTPRequestHandler):
    timeout=8  # Bound the request line and headers as well as the payload transfer.

    def log_message(self,*_args):
        pass

    def do_GET(self):
        match=REQUEST_PATH.fullmatch(self.path)
        if not match:
            self.send_error(404)
            return
        source=PullSource(int(match[1]),match[2],match[3])
        self.connection.settimeout(85)
        with tempfile.TemporaryDirectory(prefix='ci-pr-',dir=self.server.scratch) as scratch:
            try:
                revision,pack=build_pack(Path(scratch),source)
            except (ValueError,OSError,subprocess.SubprocessError):
                self.send_error(409,'exact PR source unavailable')
                return
            self.send_response(200)
            self.send_header('Content-Length',str(pack.stat().st_size))
            self.send_header('X-Sandproof-Merge',revision)
            self.send_header('X-Sandproof-Sha256',digest(pack))
            self.end_headers()
            try:
                with pack.open('rb') as stream:
                    shutil.copyfileobj(stream,self.wfile)
            except OSError:
                return
        print(f'published PR source {source.number} {revision}',flush=True)


def translated_argv(argv,environment,source,merge):
    prefix=['-c','protocol.version=2','fetch','--no-tags','--prune','--no-recurse-submodules']
    tail=['--depth=1','origin',f'+{merge}:refs/remotes/pull/{source.number}/merge']
    return (argv in (prefix+tail,prefix+['--progress']+tail)
            and environment.get('GITHUB_REPOSITORY','').casefold()==REPOSITORY.casefold()
            and environment.get('GITHUB_EVENT_NAME')=='pull_request'
            and environment.get('GITHUB_REF')==f'refs/pull/{source.number}/merge'
            and environment.get('GITHUB_SHA')==merge)


def guest_wrapper(disk):
    # Ship only the fixed-purpose argv check to the runner, with no host settings.
    import inspect
    match=inspect.getsource(translated_argv)
    return f'''#!/usr/bin/python3 -I
import os, shlex, subprocess, sys
from types import SimpleNamespace
REPOSITORY = {REPOSITORY!r}
{match}
git = '/usr/bin/git'
argv = sys.argv[1:]
if translated_argv(argv,os.environ,SimpleNamespace(number={disk.source.number!r}),{disk.merge!r}):
    origin = subprocess.run([git,'config','--local','--get','remote.origin.url'],text=True,capture_output=True)
    if origin.returncode == 0 and origin.stdout.strip().casefold() in ({URL.casefold()!r}, {URL[:-4].casefold()!r}):
        cache = {MOUNT!r}+'/repo.git'
        if not os.statvfs(cache).f_flag & os.ST_RDONLY:
            sys.exit('Sandproof source cache is not read-only')
        safe = 'safe.directory='+cache
        refs = subprocess.check_output([git,'-c',safe,'--git-dir='+cache,'rev-parse','HEAD','HEAD^{{tree}}'],text=True).splitlines()
        if refs != [{disk.merge!r},{disk.tree!r}]:
            sys.exit('Sandproof source cache identity mismatch')
        print('Sandproof CI source cache hit: '+refs[0],flush=True)
        serve = '--upload-pack=/usr/bin/git -c '+shlex.quote(safe)+' upload-pack'
        argv = ['-c','protocol.file.allow=always']+argv[:-2]+[serve,'file://'+cache,argv[-1]]
os.execv(git,[git]+argv)
'''


def download(settings,source,work):
    pack=work/'source.pack'
    headers=work/'headers'
    url=f"http://127.0.0.1:{settings['publisher_port']}"+source.request_path
    subprocess.run(['/usr/bin/curl','--fail','--silent','--noproxy','',
        '--proxy',f"socks5h://127.0.0.1:{settings['socks_port']}", '--connect-timeout','6','--max-time','110',
        '--max-filesize',str(MAX_PACK),'--dump-header',str(headers),'--output',str(pack),url],check=True,timeout=115)
    fields={name.casefold():value.strip() for row in headers.read_text().splitlines() if ':' in row
            for name,value in (row.split(':',1),)}
    revision=fields.get('x-sandproof-merge','')
    wanted=fields.get('x-sandproof-sha256','')
    if (not SHA.fullmatch(revision) or not DIGEST.fullmatch(wanted) or not 0<pack.stat().st_size<=MAX_PACK
            or fields.get('content-length')!=str(pack.stat().st_size) or digest(pack)!=wanted):
        raise ValueError('received source package differs from publisher envelope')
    return revision,pack


def prepare_disk(settings,source,work):
    revision,pack=download(settings,source,work)
    content=work/'content'
    content.mkdir(mode=0o755)
    repo=content/'repo.git'
    repo.mkdir(mode=0o755)
    git(repo,'init','--bare','--quiet')
    (repo/'shallow').write_text(revision+'\n')
    with pack.open('rb') as stream:
        git(repo,'index-pack','--stdin','--strict',stdin=stream,capture_output=True)
    tree=verify_repo(repo,source,revision)
    git(repo,'update-ref','refs/heads/pinned',revision)
    git(repo,'symbolic-ref','HEAD','refs/heads/pinned')
    git(repo,'fsck','--strict','--no-reflogs',revision,capture_output=True)
    for item in repo.rglob('*'):
        item.chmod(0o755 if item.is_dir() else 0o444)
    for directory in (repo,content):
        directory.chmod(0o755)
    published=work/'published'
    published.mkdir(mode=0o711)
    image=published/'source.iso'
    subprocess.run(['/usr/bin/prlimit',f'--fsize={MAX_DISK}','--','/usr/bin/genisoimage','-quiet','-R','-V',LABEL,
                    '-o',str(image),str(content)],check=True,timeout=25)
    image.chmod(0o444)
    record=dict(source=asdict(source),merge=revision,tree=tree,bytes=image.stat().st_size,sha256=digest(image))
    (published/'manifest.json').write_text(json.dumps(record)+'\n')
    # QEMU runs as the unprivileged VM account, outside the launcher's umask.
    published.chmod(0o711)
    return published,record


class SourceCache:
    def __init__(self,root=CACHE):
        self.root=root

    @contextmanager
    def lock(self,wait):
        attrs=self.root.lstat()
        if not stat.S_ISDIR(attrs.st_mode) or attrs.st_uid!=os.geteuid() or attrs.st_mode&0o022:
            raise ValueError('untrusted source cache directory')
        if self.root==CACHE and attrs.st_dev==Path('/').stat().st_dev:
            raise ValueError('source cache must reside on the data filesystem')
        with os.fdopen(os.open(self.root/'.lock',os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600),'rb') as stream:
            deadline=time.monotonic()+wait
            while True:
                try:
                    fcntl.flock(stream,fcntl.LOCK_EX|fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic()>=deadline:
                        raise
                    time.sleep(.2)
            yield

    def entries(self):
        entries=[]
        for path in self.root.iterdir():
            if not SHA.fullmatch(path.name) or path.is_symlink() or not path.is_dir():
                continue
            image=path/'source.iso'
            manifest=path/'manifest.json'
            attrs=image.lstat()
            if (not stat.S_ISREG(attrs.st_mode) or attrs.st_uid!=os.geteuid() or attrs.st_mode&0o222
                    or manifest.is_symlink() or not manifest.is_file()):
                raise ValueError('unexpected source cache file')
            entries.append((manifest.stat().st_mtime,path,attrs.st_size))
        return sorted(entries)

    def prune(self, reserve=0):
        # Caller holds the global lock; incomplete builds cannot be active.
        for path in self.root.glob('.build-*'):
            if not path.is_symlink() and path.is_dir() and path.stat().st_uid==os.geteuid():
                shutil.rmtree(path)
        entries=self.entries()
        used=sum(size for _,_,size in entries)
        count=len(entries)
        for modified,path,size in entries:
            if (time.time()-modified<MAX_IDLE and used+reserve<=MAX_CACHE
                    and count+bool(reserve)<=MAX_ENTRIES):
                continue
            with (path/'source.iso').open('rb') as image:
                try:
                    fcntl.flock(image,fcntl.LOCK_EX|fcntl.LOCK_NB)
                except BlockingIOError:
                    continue
                shutil.rmtree(path)
            used-=size
            count-=1
        return used+reserve<=MAX_CACHE and count+bool(reserve)<=MAX_ENTRIES

    def collect(self):
        try:
            with self.lock(0):
                self.prune()
        except BlockingIOError:
            pass

    def recently_failed(self, source, *, record=False):
        path=self.root/'failures.json'
        previous=json.loads(path.read_text()) if path.exists() else []
        now=time.time()
        live=[item for item in previous if 0<=now-item['time']<FAILURE_BACKOFF]
        identity=asdict(source)
        if record:
            live.append(dict(source=identity,time=now))
            temporary=self.root/'.failures.tmp'
            temporary.write_text(json.dumps(live[-MAX_ENTRIES:])+'\n')
            temporary.replace(path)
        return any(item['source']==identity for item in live)

    @contextmanager
    def acquire(self,source,settings):
        image=None
        try:
            with self.lock(115):
                self.prune()
                candidate=None
                for _,path,_ in reversed(self.entries()):
                    record=json.loads((path/'manifest.json').read_text())
                    if not isinstance(record,dict):
                        raise ValueError('invalid cached source manifest')
                    if record.get('source')==asdict(source):
                        candidate=path,record
                        break
                if candidate is None and not self.recently_failed(source) and self.prune(MAX_DISK):
                    try:
                        with tempfile.TemporaryDirectory(dir=self.root,prefix='.build-') as work:
                            published,record=prepare_disk(settings,source,Path(work))
                            path=self.root/record['merge']
                            if not 0<record['bytes']<=MAX_DISK or path.exists():
                                raise ValueError('invalid or conflicting source cache publication')
                            published.rename(path)
                            candidate=path,record
                    except (ValueError,OSError,subprocess.SubprocessError):
                        self.recently_failed(source,record=True)
                        raise
                if candidate is None:
                    disk=None
                else:
                    path,record=candidate
                    if not path.stat().st_mode & stat.S_IXOTH:
                        raise ValueError('cached PR source directory is inaccessible to the VM account')
                    name=path/'source.iso'
                    image=name.open('rb')
                    fcntl.flock(image,fcntl.LOCK_SH)
                    tree=record.get('tree','')
                    if (record.get('merge')!=path.name or not isinstance(tree,str) or not SHA.fullmatch(tree)
                            or record.get('bytes')!=name.stat().st_size or digest(name)!=record.get('sha256')):
                        raise ValueError('cached PR source failed identity check')
                    (path/'manifest.json').touch()
                    disk=SourceDisk(name,source,record['merge'],record['tree'])
            yield disk
        finally:
            if image is not None:
                image.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scratch',type=Path,required=True)
    parser.add_argument('--port',type=int,required=True)
    args=parser.parse_args()
    if not 1024<=args.port<=65535 or not args.scratch.is_dir():
        parser.error('publisher requires an existing scratch directory and a nonprivileged port')
    os.umask(0o077)
    attrs=args.scratch.lstat()
    if not stat.S_ISDIR(attrs.st_mode) or attrs.st_uid!=os.geteuid() or attrs.st_mode&0o077:
        parser.error('publisher scratch must be an owner-only directory')
    with os.fdopen(os.open(args.scratch/'.publisher.lock',os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600),'rb') as stream:
        fcntl.flock(stream,fcntl.LOCK_EX|fcntl.LOCK_NB)
        for path in args.scratch.glob('ci-pr-*'):
            if not path.is_symlink() and path.is_dir() and path.stat().st_uid==os.geteuid():
                shutil.rmtree(path)
        with HTTPServer(('127.0.0.1',args.port),Publisher) as server:
            server.scratch=args.scratch
            server.serve_forever()


if __name__=='__main__':
    sys.exit(main())
