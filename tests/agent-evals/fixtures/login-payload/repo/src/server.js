import http from 'node:http';

const USERS = { alice: 'secret', bob: 'hunter2' };

export function createServer() {
  return http.createServer((req, res) => {
    const send = (status, obj) => {
      res.writeHead(status, { 'content-type': 'application/json' });
      res.end(JSON.stringify(obj));
    };
    if (req.method !== 'POST' || req.url !== '/login') return send(404, { error: 'not found' });
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      let d = null;
      try { d = JSON.parse(body); } catch { /* handled below */ }
      if (!d || typeof d.username !== 'string' || typeof d.password !== 'string') return send(400, { error: 'username and password required' });
      if (USERS[d.username] !== d.password) return send(401, { error: 'bad credentials' });
      send(200, { token: 'tok-' + d.username });
    });
  });
}
