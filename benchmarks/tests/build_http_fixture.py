"""Controlled public-to-private HTTP transport for benchmark component tests.

This fixture only maps explicit synthetic user credentials to fixed tenants.
The actual private Handler/BuildStore owns all build/window/stream behavior;
this is not a production center, authority or VM acceptance substitute.
"""
import hmac
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import socket
import threading


class PublicBuildHTTP(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, backend, credentials, *, drop_first_post=False):
        self.backend = backend
        self.credentials = dict(credentials)
        self.drop_first_post = drop_first_post
        super().__init__(('127.0.0.1', 0), PublicHandler)


class PublicHandler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self):
        self.request.settimeout(5)
        keys = self.headers.get_all('X-API-Key', [])
        owner = next((tenant for key, tenant in self.server.credentials.items()
                      if len(keys) == 1 and keys[0].isascii()
                      and hmac.compare_digest(keys[0], key)), None)
        if any(name in self.headers for name in ('x-sandproof-build-key', 'x-sandproof-build-owner')):
            code, data = 400, b''  # Match the real public-only boundary.
        elif owner is None:
            code, data = 401, b''
        else:
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 <= size <= 65536:
                raise ValueError('synthetic fixture request bound exceeded')
            body = self.rfile.read(size) if size else None
            headers = {'x-sandproof-build-key': self.server.backend.backend_key,
                       'x-sandproof-build-owner': str(owner)}
            for name in ('Content-Type', 'Content-Length', 'Idempotency-Key'):
                if name in self.headers:
                    headers[name] = self.headers[name]
            # Never forward the ordinary key or caller-supplied private owner.
            connection = http.client.HTTPConnection('127.0.0.1', self.server.backend.server_port, timeout=5)
            try:
                connection.request(self.command, self.path, body, headers)
                response = connection.getresponse()
                code, data = response.status, response.read(65537)
                if len(data) > 65536:
                    raise ValueError('synthetic fixture response bound exceeded')
            finally:
                connection.close()
            if (self.server.drop_first_post and self.command == 'POST'
                    and self.path == '/sandproof/template-builds'):
                self.server.drop_first_post = False
                # The real private service has durably accepted the original
                # request before the public response is deliberately lost.
                self.close_connection = True
                self.request.shutdown(socket.SHUT_RDWR)
                return
        self.send_response(code)
        if data:
            self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_POST = respond
    do_GET = respond
    do_PUT = respond


def serve_http(test, server):
    """Retain and reap each exact fixture server before its store closes."""
    thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.01}, daemon=True)
    thread.start()

    def close():
        server.shutdown()
        thread.join(2)
        server.server_close()
        test.assertFalse(thread.is_alive())

    test.addCleanup(close)
    return server
