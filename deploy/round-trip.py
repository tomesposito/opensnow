#!/usr/bin/env python3
"""Fail on any HTTP or data mismatch. Uses a fresh tenant; never prints its token."""
import json
import os
import ssl
import sys
import time
import urllib.request
import uuid

base = sys.argv[1].rstrip('/')
context = ssl.create_default_context(cafile=os.environ.get('OPENSNOW_TEST_CA'))

def request(path, data=None, token=None):
    headers = {'Content-Type': 'application/json'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(base + path, headers=headers,
                                 data=None if data is None else json.dumps(data).encode())
    with urllib.request.urlopen(req, context=context, timeout=30) as response:
        return json.load(response)

for attempt in range(60):
    try:
        assert request('/health') == {'status': 'ok'}
        break
    except Exception:
        if attempt == 59:
            raise
        time.sleep(1)
print('PASS /health: {"status":"ok"}', flush=True)
provisioned = request('/api/provision', {
    'name': 'deploy_' + uuid.uuid4().hex,
    'admin_login': 'deploy@example.test',
    'admin_password': uuid.uuid4().hex,
})
token = provisioned['token']
assert isinstance(token, str) and token
print('PASS /api/provision: token returned (redacted)', flush=True)
for sql in ('CREATE TABLE deploy_check (id BIGINT NOT NULL)',
            'INSERT INTO deploy_check VALUES (19)',
            'SELECT id FROM deploy_check'):
    result = request('/api/execute', {'sql': sql}, token)
    if sql.startswith('SELECT'):
        assert result['rows'] == [['19']], result
    print('PASS /api/execute: ' + sql, flush=True)
print('PASS round-trip', flush=True)
