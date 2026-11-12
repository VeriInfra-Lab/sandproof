"""Bounded single-inflight SDK workload; failures are observations, never retried."""
import json
import re
import threading
import time

class PeerProbe:
    def __init__(self, client, sdk, output, nonce):
        if not isinstance(nonce,str) or not re.fullmatch('[0-9a-f]{1,64}',nonce):
            raise ValueError('bounded synthetic hexadecimal marker required')
        self.client, self.sdk, self.output, self.nonce = client, sdk, output, nonce
        self.stop_event = threading.Event()
        self.started = threading.Event()
        self.label = 'setup'
        self.rows = []
        self.failure = None
        self.thread = threading.Thread(target=self.run, name='peer-sdk-probe')

    def measure(self, kind, action):
        row = dict(phase=self.label, kind=kind, started=time.monotonic(), wall=time.time())
        try:
            action()
            row['ok'] = True
        except Exception as error:
            status=getattr(error,'actual',getattr(error,'status_code',None))
            if not isinstance(status,int):
                match=re.match(r'^([1-5][0-9]{2}):(?: |$)',str(error))
                status=int(match[1]) if match else None
            row.update(ok=False,error_type=type(error).__name__,status=status)
        row['seconds'] = time.monotonic() - row['started']
        self.rows.append(row)
        with self.output.open('a') as log:
            log.write(json.dumps(row, sort_keys=True)+'\n')
        return row['ok']

    def run(self):
        try:
            self._run()
        except BaseException as error:
            self.failure = type(error).__name__

    def _run(self):
        self.started.set()
        sequence = 0
        while not self.stop_event.is_set():
            if sequence >= 512:
                raise RuntimeError('finite peer probe sample budget exhausted')
            sequence += 1
            start = time.monotonic()
            def query():
                view = self.client.query()
                assert view['phase'] == 'running', view['phase']
            self.measure('query', query)
            marker = f'{self.nonce}:{sequence}'
            path = f'/tmp/peer-probe-{sequence}'
            def command():
                result = self.sdk.commands.run(f"printf '%s' '{marker}' > {path}; cat {path}", timeout=20)
                assert result.exit_code == 0 and result.stdout == marker, (result.exit_code, result.stdout)
            if self.measure('command', command):
                def read():
                    assert self.sdk.files.read(path) == marker, 'acknowledged command marker differs'
                self.measure('file-read', read)
            self.stop_event.wait(max(0, 1 - (time.monotonic()-start)))

    def start(self):
        self.thread.start()
        assert self.started.wait(5)

    def stop(self):
        self.stop_event.set()
        self.thread.join(100)
        assert not self.thread.is_alive(), 'original SDK probe has not exited'
        assert self.failure is None, self.failure

    def summary(self):
        results = {}
        for row in self.rows:
            key = row['phase'] + '/' + row['kind']
            results.setdefault(key, []).append(row)
        return {key: dict(attempts=len(rows), failures=sum(not r['ok'] for r in rows),
                          median_seconds=sorted(r['seconds'] for r in rows)[len(rows)//2],
                          p95_seconds=sorted(r['seconds'] for r in rows)[max(0, (95*len(rows)+99)//100-1)],
                          maximum_seconds=max(r['seconds'] for r in rows))
                for key, rows in results.items()}
