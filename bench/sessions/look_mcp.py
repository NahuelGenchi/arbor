"""`arbor look` as a tool of its own, for the tool arm of the whole-session A/B (ab.py): an MCP server that OpenCode
starts in the cell, from the settings in the key's home.

usage: python3 look_mcp.py TOOL.json

It speaks MCP over its input and output, one JSON-RPC message per line, and offers one tool, named and described as
TOOL.json says (tasks.TOOL). The tool takes a list of queries and answers with what `arbor look Q1 + Q2 + ...` prints,
run in the server's working folder, which is the agent's. It reads nothing else and keeps nothing.
"""
import json
import shlex
import subprocess
import sys


def look(queries):
    """(what `arbor look` printed for the queries, True when arbor could not answer at all)."""
    if isinstance(queries, str):
        queries = [queries]
    args = []
    try:
        for q in queries or []:
            words = shlex.split(str(q))
            if words[:2] == ['arbor', 'look']:  # the shell command written out, as the agent may
                words = words[2:]
            args += (['+'] if args and words else []) + words
    except ValueError as e:
        return f'cannot read the queries: {e}', True
    r = subprocess.run(['arbor', 'look', *args], stdin=subprocess.DEVNULL, capture_output=True)
    # 1 means some query found nothing, which its answer says; anything else is arbor refusing the call
    return (r.stdout + r.stderr).decode(errors='replace'), r.returncode not in (0, 1)


def reply(tool, req):
    method, params = req.get('method'), req.get('params') or {}
    if method == 'initialize':
        return {'protocolVersion': params.get('protocolVersion') or '2025-06-18', 'capabilities': {'tools': {}},
                'serverInfo': {'name': 'arbor', 'version': '0'}}
    if method == 'ping':
        return {}
    if method == 'tools/list':
        return {'tools': [tool]}
    if method == 'tools/call' and params.get('name') == tool['name']:
        text, error = look((params.get('arguments') or {}).get('queries'))
        return {'content': [{'type': 'text', 'text': text}], 'isError': error}
    raise LookupError(f'no method {method} {params.get("name") or ""}'.strip())


def main():
    with open(sys.argv[1]) as f:
        tool = json.load(f)
    for line in sys.stdin:
        try:
            req = json.loads(line)
        except ValueError:
            continue
        if not isinstance(req, dict) or 'id' not in req:  # a notification: nothing to answer
            continue
        try:
            msg = {'jsonrpc': '2.0', 'id': req['id'], 'result': reply(tool, req)}
        except LookupError as e:
            msg = {'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -32601, 'message': str(e)}}
        sys.stdout.write(json.dumps(msg) + '\n')
        sys.stdout.flush()


if __name__ == '__main__':
    main()
