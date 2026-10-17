"""Times connections to a server that answers each request briefly: made directly, or through an HTTPS proxy.

  python3 -I connect.py direct|proxy HOST:PORT N WARM

Each connection opens, sends one request and reads the answer to its end, which must be `pong`. Through the proxy it
first asks for a tunnel to HOST:PORT (`CONNECT`), as a tool does for HTTPS; the proxy is the one HTTPS_PROXY names. The
arbor section runs this inside a cell, where that is the proxy the cell's key opens, and outside any cell, straight to
the same server. Prints the times of the N connections after WARM untimed ones, in ms, as a JSON list.
"""
import json
import os
import socket
import sys
import time
from urllib.parse import urlsplit

REQUEST = b'GET / HTTP/1.1\r\nHost: arbor-bench\r\n\r\n'


def head(s):
    """Reads up to the end of the head of an HTTP answer, which the proxy sends alone."""
    data = b''
    while b'\r\n\r\n' not in data:
        chunk = s.recv(4096)
        if not chunk:
            raise RuntimeError(f'the proxy closed the connection: {data[:120]!r}')
        data += chunk
    return data


def once(target, proxy):
    host, port = target.rsplit(':', 1)
    s = socket.create_connection(proxy or (host, int(port)))
    with s:
        if proxy:
            s.sendall(f'CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n'.encode())
            answer = head(s)
            if not answer.startswith(b'HTTP/1.1 200 '):
                raise RuntimeError(f'the proxy answered {answer.splitlines()[0].decode(errors="replace")}')
        s.sendall(REQUEST)
        answer = b''
        while chunk := s.recv(4096):
            answer += chunk
    if not answer.endswith(b'\r\n\r\npong\n'):
        raise RuntimeError(f'the server answered {answer[:120]!r}')


def main():
    mode, target, n, warm = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    proxy = None
    if mode == 'proxy':
        url = urlsplit(os.environ.get('HTTPS_PROXY', ''))
        if not url.hostname or not url.port:
            sys.exit('connect.py: HTTPS_PROXY names no proxy')
        proxy = (url.hostname, url.port)
    times = []
    for _ in range(warm + n):
        t0 = time.perf_counter_ns()
        once(target, proxy)
        times.append((time.perf_counter_ns() - t0) / 1e6)
    print(json.dumps([round(t, 4) for t in times[warm:]]))


if __name__ == '__main__':
    main()
