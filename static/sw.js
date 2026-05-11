const CACHE = 'gpx-tools-v1';
const SHELL = [
  '/',
  '/manifest.json',
  '/icon-192.png',
  '/icon-512.png',
  '/static/themes.css',
  '/static/theme-switcher.js',
];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE).then((c) =>
      Promise.all(
        SHELL.map((url) =>
          fetch(url, { cache: 'reload' })
            .then((r) => (r.ok ? c.put(url, r) : null))
            .catch(() => null)
        )
      )
    )
  );
  self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    )
  );
  self.clients.claim();
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  const p = url.pathname;
  if (
    p.includes('/api/') ||
    p.startsWith('/auth/') ||
    p.includes('/webhook/') ||
    p.startsWith('/share/') ||
    p.endsWith('/ws') ||
    p.startsWith('/generate') ||
    p.startsWith('/merge')
  ) {
    return;
  }

  e.respondWith(
    fetch(req)
      .then((res) => {
        if (res.ok && (req.mode === 'navigate' || SHELL.includes(p))) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        }
        return res;
      })
      .catch(() => caches.match(req).then((r) => r || caches.match('/')))
  );
});
