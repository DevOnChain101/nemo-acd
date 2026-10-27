import http from 'node:http';

const ITEMS = Array.from({ length: 25 }, (_, i) => ({ id: i + 1 }));

export function paginate(items, page, size) {
  const start = (page - 1) * size;
  return items.slice(start, start + size);
}

export function createServer() {
  return http.createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    if (req.method !== 'GET' || url.pathname !== '/items') {
      res.writeHead(404).end();
      return;
    }
    const page = Math.max(1, parseInt(url.searchParams.get('page') || '1', 10) || 1);
    const size = Math.max(1, parseInt(url.searchParams.get('size') || '10', 10) || 10);
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ page, size, total: ITEMS.length, items: paginate(ITEMS, page, size) }));
  });
}
