use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Json, Multipart, Path as AxumPath, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use gpx_to_graph::{generate, GeneratedOutput, GraphOptions};
use serde_json::{json, Value};
use tokio::sync::Mutex;

struct MergeSession {
    files: Vec<(Option<String>, Vec<u8>)>,
    created: Instant,
}

type MergeSessions = Arc<Mutex<HashMap<String, MergeSession>>>;

const SHARE_TTL_SECS: u64 = 30 * 24 * 3600;

fn share_dir() -> PathBuf {
    std::env::var("GPX_SHARE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/gpx_to_graph_share"))
}

fn random_id() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::io::Read;
    let mut buf = [0u8; 12];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((n >> (i * 8)) & 0xff) as u8;
        }
    }
    URL_SAFE_NO_PAD.encode(buf)
}

fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_safe_filename(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Resolve the public base URL (e.g. `https://example.com`).
///
/// `PUBLIC_BASE_URL` env wins if set. Otherwise we derive it from request
/// headers (`X-Forwarded-Proto` / `X-Forwarded-Host` for reverse proxies,
/// then `Host`). When the proto can't be determined we default to `https`
/// for any non-local host, so links to e.g. `gpx.studio` stay HTTPS-only
/// even behind a Caddy/Nginx that forgets `X-Forwarded-Proto`.
fn public_base_url(headers: &HeaderMap) -> String {
    if let Ok(env_url) = std::env::var("PUBLIC_BASE_URL") {
        let trimmed = env_url.trim_end_matches('/').to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let host_no_port = host.split(':').next().unwrap_or(host);
    let is_local = host_no_port == "localhost"
        || host_no_port == "::1"
        || host_no_port == "0.0.0.0"
        || host_no_port.starts_with("127.");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| {
            if is_local {
                "http".to_string()
            } else {
                "https".to_string()
            }
        });
    format!("{scheme}://{host}")
}

const FORM_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>GPX Tools</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:opsz,wght@12..96,400;12..96,500;12..96,600;12..96,700;12..96,800&family=Fraunces:ital,opsz,wght@0,9..144,400;0,9..144,700;0,9..144,800;1,9..144,400;1,9..144,700&family=Space+Mono:wght@400;700&display=swap" rel="stylesheet">
<link rel="stylesheet" href="/static/recents.css">
<style>
  :root {
    --paper: #f2e9d4; --paper-2: #e8dec0; --ink: #0e1424; --ink-soft: #2a2f3e;
    --muted: #79654a; --carmine: #b8242a; --teal: #14707a; --mustard: #d9a326; --moss: #476b2e;
    --rule: rgba(14,20,36,.18);
    --f-display: 'Fraunces', Georgia, serif;
    --f-body: 'Bricolage Grotesque', system-ui, sans-serif;
    --f-mono: 'Space Mono', monospace;
  }
  *, *::before, *::after { box-sizing: border-box; }
  html { -webkit-font-smoothing: antialiased; text-rendering: optimizeLegibility; }
  body {
    font-family: var(--f-body);
    background: var(--paper);
    color: var(--ink);
    margin: 0;
    padding: 2rem 1rem;
    line-height: 1.5;
  }
  .grain {
    pointer-events: none; position: fixed; inset: 0; z-index: 50;
    opacity: .22; mix-blend-mode: multiply;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='200' height='200'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0.055 0 0 0 0 0.078 0 0 0 0 0.141 0 0 0 0.5 0'/></filter><rect width='100%25' height='100%25' filter='url(%23n)'/></svg>");
  }
  .container {
    max-width: 900px;
    margin: 0 auto;
    position: relative;
    z-index: 1;
  }
  h1 {
    font-family: var(--f-display);
    font-weight: 800;
    font-size: clamp(2rem, 5vw, 2.8rem);
    letter-spacing: -0.02em;
    margin: 0 0 0.25rem;
    font-variation-settings: "opsz" 72;
  }
  .subtitle {
    font-family: var(--f-mono);
    font-size: 0.72rem;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
    margin: 0 0 2rem;
  }
  .card {
    background: var(--paper-2);
    border: 1px solid var(--ink);
    padding: 2rem;
  }
  .file-section {
    margin-bottom: 1.5rem;
  }
  .file-section label {
    display: block;
    font-weight: 600;
    margin-bottom: 0.5rem;
    font-size: 0.95rem;
  }
  .file-section input[type="file"] {
    display: block;
    width: 100%;
    padding: 0.75rem;
    border: 2px dashed var(--rule);
    background: var(--paper);
    cursor: pointer;
    font-size: 0.9rem;
    font-family: var(--f-body);
  }
  .file-section input[type="file"]:hover {
    border-color: var(--ink);
  }
  .section-title {
    font-family: var(--f-mono);
    font-size: 0.72rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
    margin: 1.5rem 0 1rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--rule);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }
  @media (max-width: 600px) {
    .grid { grid-template-columns: 1fr; }
  }
  .field {
    display: flex;
    flex-direction: column;
  }
  .field label {
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--ink-soft);
    margin-bottom: 0.35rem;
  }
  .field input[type="number"],
  .field input[type="text"] {
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--rule);
    font-size: 0.9rem;
    font-family: var(--f-body);
    background: var(--paper);
    transition: border-color 0.15s;
  }
  .field input:focus {
    outline: none;
    border-color: var(--ink);
    box-shadow: 0 0 0 3px rgba(14,20,36,0.08);
  }
  .checkbox-field {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding-top: 1.5rem;
  }
  .checkbox-field input[type="checkbox"] {
    width: 18px;
    height: 18px;
    cursor: pointer;
    accent-color: var(--carmine);
  }
  .checkbox-field label {
    font-size: 0.9rem;
    cursor: pointer;
    color: var(--ink);
  }
  .hint {
    font-size: 0.75rem;
    color: var(--muted);
    margin-top: 0.25rem;
  }
  .submit-section {
    margin-top: 2rem;
    text-align: right;
  }
  button[type="submit"] {
    background: var(--carmine);
    color: var(--paper);
    border: none;
    padding: 0.75rem 2rem;
    font-family: var(--f-mono);
    font-size: 0.82rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    transition: background 0.15s;
  }
  button[type="submit"]:hover {
    background: #961e22;
  }
  .site-nav {
    display: flex;
    align-items: center;
    gap: 0;
    border-bottom: 2px solid var(--ink);
    margin-bottom: 2rem;
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
  }
  .nav-brand {
    font-family: var(--f-display);
    font-weight: 800;
    font-size: 1.1rem;
    color: var(--ink);
    text-decoration: none;
    padding: 0.6rem 1.25rem 0.6rem 0;
    margin-right: 0.5rem;
    border-right: 1px solid var(--rule);
    white-space: nowrap;
    font-variation-settings: "opsz" 72;
  }
  .nav-link {
    display: inline-block;
    padding: 0.6rem 1.1rem;
    font-family: var(--f-mono);
    font-size: 0.72rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
    text-decoration: none;
    border-bottom: 3px solid transparent;
    margin-bottom: -2px;
    transition: color 0.15s, border-color 0.15s;
    white-space: nowrap;
  }
  .nav-link:hover { color: var(--ink); }
  .nav-link.active {
    color: var(--carmine);
    border-bottom-color: var(--carmine);
  }
  .local-tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--rule);
    margin-bottom: 1.5rem;
  }
  .local-tab {
    background: transparent;
    border: none;
    padding: 0.5rem 1.25rem;
    font-family: var(--f-mono);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    transition: color 0.15s, border-color 0.15s;
  }
  .local-tab:hover { color: var(--ink); }
  .local-tab.active {
    color: var(--ink);
    border-bottom-color: var(--ink);
  }
  .panel { display: none; }
  .panel.active { display: block; }
  .status-line {
    margin-top: 1rem;
    font-weight: 600;
    font-size: 0.95rem;
  }
  .status-line.info { color: var(--teal); }
  .status-line.ok { color: var(--moss); }
  .status-line.err { color: var(--carmine); }
  .file-section.dragging input[type="file"] {
    border-color: var(--ink);
    background: var(--paper-2);
  }
</style>
<link rel="stylesheet" href="/static/themes.css">
<script>document.documentElement.dataset.theme=localStorage.getItem('gpx-theme')||'golden-hour'</script>
<script src="/static/theme-switcher.js" defer></script>
</head>
<body>
<div class="grain" aria-hidden="true"></div>
<aside id="recentsSidebar" class="recents-sidebar" aria-label="Recent routes" hidden>
  <div class="rx-header">
    <span class="rx-title">Recent routes</span>
    <button type="button" id="recentsClear" class="rx-clear" title="Clear all">×</button>
  </div>
  <ol id="recentsList" class="rx-list"></ol>
</aside>
<div class="container">
  <nav class="site-nav">
    <a href="/" class="nav-brand">GPX Tools</a>
    <a href="/" class="nav-link active">Graph</a>
    <a href="/toolkit" class="nav-link">Toolkit</a>
    <a href="/meteo" class="nav-link">Meteo</a>
    <a href="/ravito" class="nav-link">Ravito</a>
    <a href="/trace" class="nav-link">Trace</a>
    <a href="/stats" class="nav-link">Stats</a>
    <a href="/col" class="nav-link">Col</a>
    <a href="/trip" class="nav-link">Trip</a>
  </nav>

  <div class="local-tabs">
    <button type="button" class="local-tab active" data-target="graph">Generate</button>
  </div>

  <div id="panel-graph" class="panel active">
    <div class="card">
      <form method="post" action="/generate" enctype="multipart/form-data">
        <div class="file-section">
          <label for="gpx_file">GPX File</label>
          <input type="file" id="gpx_file" name="gpx_file" accept=".gpx" required>
        </div>

        <div class="section-title">Graph Settings</div>
        <div class="grid">
          <div class="field">
            <label for="km_step">KM Step (grid lines)</label>
            <input type="number" id="km_step" name="km_step" value="10" step="any" min="0.1">
          </div>
          <div class="field">
            <label for="km_label_step">KM Label Step</label>
            <input type="number" id="km_label_step" name="km_label_step" value="25" step="any" min="0.1">
          </div>
          <div class="field">
            <label for="km_label_scale">KM Label Scale</label>
            <input type="number" id="km_label_scale" name="km_label_scale" value="5" min="1" max="8">
          </div>
          <div class="field">
            <label for="climb_min_gain">Min Climb Gain (m)</label>
            <input type="number" id="climb_min_gain" name="climb_min_gain" value="30" step="any" min="0">
          </div>
          <div class="field">
            <label for="split">Split every N km</label>
            <input type="number" id="split" name="split" value="" step="any" min="0.1" placeholder="Leave empty to disable">
            <span class="hint">Leave empty to generate a single image</span>
          </div>
          <div class="field">
            <label for="checkpoint_filter">Checkpoint Filter</label>
            <input type="text" id="checkpoint_filter" name="checkpoint_filter" value="" placeholder="Optional text filter">
            <span class="hint">Keep only checkpoints matching this text</span>
          </div>
          <div class="checkbox-field">
            <input type="checkbox" id="mirror" name="mirror" value="on">
            <label for="mirror">Mirror horizontally</label>
          </div>
        </div>

        <div class="submit-section">
          <button type="submit">Generate Profile</button>
        </div>
      </form>
    </div>
  </div>


</div>
<script>
  document.querySelectorAll('.local-tab').forEach(function (tab) {
    tab.addEventListener('click', function () {
      document.querySelectorAll('.local-tab').forEach(function (t) { t.classList.remove('active'); });
      document.querySelectorAll('.panel').forEach(function (p) { p.classList.remove('active'); });
      tab.classList.add('active');
      document.getElementById('panel-' + tab.dataset.target).classList.add('active');
    });
  });

  function enableDropZone(input) {
    var zone = input.parentElement;
    if (!zone) return;
    ['dragenter', 'dragover'].forEach(function (ev) {
      zone.addEventListener(ev, function (e) { e.preventDefault(); zone.classList.add('dragging'); });
    });
    zone.addEventListener('dragleave', function (e) {
      if (!zone.contains(e.relatedTarget)) zone.classList.remove('dragging');
    });
    zone.addEventListener('drop', function (e) {
      e.preventDefault();
      zone.classList.remove('dragging');
      if (!e.dataTransfer || !e.dataTransfer.files) return;
      var dt = new DataTransfer();
      var accepted = 0;
      for (var i = 0; i < e.dataTransfer.files.length; i++) {
        var f = e.dataTransfer.files[i];
        var ok = f.name.toLowerCase().endsWith('.gpx') || f.type === 'application/gpx+xml' || f.type === '';
        if (ok) { dt.items.add(f); accepted++; break; }
      }
      if (accepted === 0) return;
      input.files = dt.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });
  }
  var gpxInput = document.getElementById('gpx_file');
  if (gpxInput) enableDropZone(gpxInput);
</script>
<script src="/static/recents.js" defer></script>
</body>
</html>"##;

async fn form_page() -> Html<&'static str> {
    Html(FORM_HTML)
}

// ---------------------------------------------------------------------------
// Theme switcher — served as static assets, referenced by all pages.
// ---------------------------------------------------------------------------

const THEMES_CSS: &str = include_str!("../../static/themes.css");
const THEME_JS: &str = include_str!("../../static/theme-switcher.js");

async fn static_themes_css() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(THEMES_CSS))
        .expect("valid response")
}

async fn static_theme_js() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/javascript; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(THEME_JS))
        .expect("valid response")
}

// ---------------------------------------------------------------------------
// Recents sidebar — small LocalStorage-backed list of recently viewed shares.
// Served as static assets so the share page and the form page reuse them.
// ---------------------------------------------------------------------------

const RECENTS_CSS: &str = r##"
.recents-sidebar {
  position: fixed;
  top: 2rem;
  left: 1rem;
  width: 220px;
  max-height: calc(100vh - 4rem);
  overflow-y: auto;
  background: var(--paper-2, #e8dec0);
  border: 1px solid var(--ink, #0e1424);
  padding: 0.75rem 1rem;
  z-index: 40;
  font-family: var(--f-body, 'Bricolage Grotesque', system-ui, sans-serif);
}
.recents-sidebar[hidden] { display: none; }
.rx-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.5rem;
  border-bottom: 1px solid var(--rule, rgba(14,20,36,.18));
  padding-bottom: 0.35rem;
}
.rx-title {
  font-family: var(--f-mono, 'Space Mono', monospace);
  font-weight: 700;
  font-size: 0.65rem;
  color: var(--muted, #79654a);
  text-transform: uppercase;
  letter-spacing: 0.08em;
}
.rx-clear {
  background: transparent;
  border: none;
  color: var(--muted, #79654a);
  cursor: pointer;
  font-size: 1.1rem;
  line-height: 1;
  padding: 0 0.35rem;
}
.rx-clear:hover { color: var(--ink, #0e1424); }
.rx-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.rx-item a {
  display: block;
  text-decoration: none;
  padding: 0.45rem 0.55rem;
  transition: background 0.12s;
  border: 1px solid transparent;
}
.rx-item a:hover {
  background: var(--paper, #f2e9d4);
  border-color: var(--rule, rgba(14,20,36,.18));
}
.rx-item.active a {
  background: var(--paper, #f2e9d4);
  border-color: var(--ink, #0e1424);
}
.rx-item .rx-itm-title {
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--carmine, #b8242a);
}
.rx-item .rx-itm-sub {
  font-size: 0.72rem;
  color: var(--muted, #79654a);
  margin-top: 0.1rem;
}
@media (max-width: 1250px) {
  .recents-sidebar {
    position: static;
    width: auto;
    max-width: 900px;
    margin: 0 auto 1.25rem;
  }
}
"##;

const RECENTS_JS: &str = r##"
(function () {
  var KEY = 'gpxRecents';
  var MAX = 12;
  function load() {
    try {
      var raw = localStorage.getItem(KEY);
      if (!raw) return [];
      var v = JSON.parse(raw);
      return Array.isArray(v) ? v : [];
    } catch (e) { return []; }
  }
  function save(list) {
    try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX))); } catch (e) {}
  }
  function relTime(ts) {
    if (!ts) return '';
    var diff = (Date.now() / 1000) - ts;
    if (diff < 60) return 'just now';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 2592000) return Math.floor(diff / 86400) + 'd ago';
    var d = new Date(ts * 1000);
    return isNaN(d) ? '' : d.toLocaleDateString();
  }
  function render(list) {
    var sidebar = document.getElementById('recentsSidebar');
    var ul = document.getElementById('recentsList');
    if (!sidebar || !ul) return;
    if (!list.length) { sidebar.hidden = true; return; }
    sidebar.hidden = false;
    var currentId = (window.__recentCurrent && window.__recentCurrent.id) || null;
    ul.innerHTML = '';
    function escape(s) {
      return String(s == null ? '' : s)
        .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
    }
    list.forEach(function (item) {
      var li = document.createElement('li');
      li.className = 'rx-item' + (item.id === currentId ? ' active' : '');
      var a = document.createElement('a');
      a.href = '/share/' + encodeURIComponent(item.id);
      var km = (item.total_km != null) ? Number(item.total_km).toFixed(1) + ' km' : '';
      var title = (item.name && String(item.name).trim()) ? String(item.name).trim() : (km || 'Route');
      var climbs = (item.num_climbs != null) ? (item.num_climbs + ' climbs') : '';
      var cps = (item.num_checkpoints != null) ? (item.num_checkpoints + ' cps') : '';
      var subParts = [];
      if (item.name && km) subParts.push(km);
      subParts.push(relTime(item.created_at));
      if (cps) subParts.push(cps);
      if (climbs) subParts.push(climbs);
      a.innerHTML =
        '<div class="rx-itm-title">' + escape(title) + '</div>' +
        '<div class="rx-itm-sub">' + subParts.filter(Boolean).join(' · ') + '</div>';
      li.appendChild(a);
      ul.appendChild(li);
    });
  }
  function addCurrent(current) {
    var list = load();
    list = list.filter(function (x) { return x.id !== current.id; });
    list.unshift(current);
    save(list);
    return list;
  }
  function init() {
    var clearBtn = document.getElementById('recentsClear');
    if (clearBtn) {
      clearBtn.addEventListener('click', function () {
        if (confirm('Clear recent routes?')) { save([]); render([]); }
      });
    }
    var list = load();
    if (window.__recentCurrent && window.__recentCurrent.id) {
      list = addCurrent(window.__recentCurrent);
    }
    render(list);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
"##;

async fn static_recents_css() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(RECENTS_CSS))
        .expect("valid response")
}

async fn static_recents_js() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/javascript; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(RECENTS_JS))
        .expect("valid response")
}

fn error_page(message: &str) -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Error - GPX to Graph</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:wght@400;600;700&family=Fraunces:opsz,wght@9..144,700;9..144,800&family=Space+Mono:wght@400;700&display=swap" rel="stylesheet">
<style>
  :root {{
    --paper: #f2e9d4; --paper-2: #e8dec0; --ink: #0e1424;
    --muted: #79654a; --carmine: #b8242a;
    --rule: rgba(14,20,36,.18);
    --f-display: 'Fraunces', Georgia, serif;
    --f-body: 'Bricolage Grotesque', system-ui, sans-serif;
    --f-mono: 'Space Mono', monospace;
  }}
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: var(--f-body);
    background: var(--paper);
    color: var(--ink);
    margin: 0;
    padding: 2rem 1rem;
  }}
  .container {{
    max-width: 900px;
    margin: 0 auto;
  }}
  h1 {{
    font-family: var(--f-display);
    font-size: 1.75rem;
    font-weight: 700;
    color: var(--carmine);
    margin: 0 0 1rem;
    font-variation-settings: "opsz" 72;
  }}
  .card {{
    background: var(--paper-2);
    border: 1px solid var(--ink);
    padding: 2rem;
  }}
  .error-message {{
    background: var(--paper);
    border: 1px solid var(--carmine);
    padding: 1rem 1.25rem;
    color: var(--carmine);
    font-size: 0.95rem;
    margin-bottom: 1.5rem;
    white-space: pre-wrap;
    word-break: break-word;
  }}
  a {{
    color: var(--ink);
    text-decoration: none;
    font-weight: 600;
    border-bottom: 1px solid var(--rule);
  }}
  a:hover {{
    border-bottom-color: var(--ink);
  }}
</style>
</head>
<body>
<div class="container">
  <div class="card">
    <h1>Generation Failed</h1>
    <div class="error-message">{}</div>
    <a href="/">&larr; Try again</a>
  </div>
</div>
</body>
</html>"#,
        html_escape(message)
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn generate_handler(mut multipart: Multipart) -> Response {
    let mut gpx_bytes: Option<Vec<u8>> = None;
    let mut km_step: Option<f64> = None;
    let mut km_label_step: Option<f64> = None;
    let mut km_label_scale: Option<i32> = None;
    let mut mirror = false;
    let mut checkpoint_filter: Option<String> = None;
    let mut climb_min_gain: Option<f64> = None;
    let mut split: Option<f64> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "gpx_file" => {
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    const MAX_FILE: usize = 100 * 1024 * 1024;
                    if data.len() > MAX_FILE {
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("GPX file is too large ({:.1} MB, max 100 MB).",
                                    data.len() as f64 / (1024.0 * 1024.0)),
                        )
                            .into_response();
                    }
                    gpx_bytes = Some(data.to_vec());
                }
            }
            "km_step" => {
                if let Ok(text) = field.text().await {
                    km_step = text.trim().parse().ok();
                }
            }
            "km_label_step" => {
                if let Ok(text) = field.text().await {
                    km_label_step = text.trim().parse().ok();
                }
            }
            "km_label_scale" => {
                if let Ok(text) = field.text().await {
                    km_label_scale = text.trim().parse().ok();
                }
            }
            "mirror" => {
                // Checkbox: if the field is present, it's checked
                mirror = true;
                // Consume the field body so axum doesn't error
                let _ = field.bytes().await;
            }
            "checkpoint_filter" => {
                if let Ok(text) = field.text().await {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        checkpoint_filter = Some(trimmed);
                    }
                }
            }
            "climb_min_gain" => {
                if let Ok(text) = field.text().await {
                    climb_min_gain = text.trim().parse().ok();
                }
            }
            "split" => {
                if let Ok(text) = field.text().await {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        split = trimmed.parse().ok();
                    }
                }
            }
            _ => {
                // Unknown field, skip
                let _ = field.bytes().await;
            }
        }
    }

    let gpx_bytes = match gpx_bytes {
        Some(b) => b,
        None => return error_page("No GPX file was uploaded.").into_response(),
    };

    let opts = GraphOptions {
        km_step: km_step.unwrap_or(10.0),
        km_label_step: km_label_step.unwrap_or(25.0),
        km_label_scale: km_label_scale.unwrap_or(5),
        mirror,
        checkpoint_filter,
        climb_min_gain: climb_min_gain.unwrap_or(30.0),
        split,
    };

    let gpx_bytes_for_task = gpx_bytes.clone();
    let result = tokio::task::spawn_blocking(move || {
        let reader = Cursor::new(gpx_bytes_for_task);
        generate(reader, &opts)
    })
    .await;

    let output: GeneratedOutput = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return error_page(&format!("{e:#}")).into_response(),
        Err(e) => return error_page(&format!("Task failed: {e}")).into_response(),
    };

    match save_share(&gpx_bytes, &output).await {
        Ok(id) => Redirect::to(&format!("/share/{id}")).into_response(),
        Err(e) => error_page(&format!("Failed to save share: {e}")).into_response(),
    }
}

/// Best-effort GPX route name: `<metadata><name>` first, then the first
/// `<trk><name>`, then None. Trimmed, length-capped.
fn extract_gpx_name(data: &[u8]) -> Option<String> {
    fn clean(s: String) -> Option<String> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        Some(t.chars().take(200).collect())
    }

    if let (Some(start), Some(end)) = (
        find_after(data, b"<metadata", 0),
        find_after(data, b"</metadata>", 0),
    )
        && end > start
        && let Some(n) = extract_tag_text(&data[start..end], b"name").and_then(clean)
    {
        return Some(n);
    }
    if let Some(trk) = find_after(data, b"<trk", 0)
        && let Some(gt) = find_after(data, b">", trk)
    {
        let rest = &data[gt + 1..];
        if let Some(n) = extract_tag_text(rest, b"name").and_then(clean) {
            return Some(n);
        }
    }
    None
}

async fn save_share(gpx_bytes: &[u8], output: &GeneratedOutput) -> Result<String, String> {
    let id = random_id();
    let dir = share_dir().join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create_dir_all: {e}"))?;

    tokio::fs::write(dir.join("source.gpx"), gpx_bytes)
        .await
        .map_err(|e| format!("write source.gpx: {e}"))?;

    let name = extract_gpx_name(gpx_bytes);

    let mut images_meta: Vec<Value> = Vec::new();
    for (i, (label, bytes)) in output.graph_images.iter().enumerate() {
        let filename = format!("img_{i}.png");
        tokio::fs::write(dir.join(&filename), bytes)
            .await
            .map_err(|e| format!("write {filename}: {e}"))?;
        images_meta.push(json!({ "label": label, "filename": filename }));
    }

    let climb_stats_filename = if let Some(bytes) = &output.climb_stats {
        let filename = "climbs.png".to_string();
        tokio::fs::write(dir.join(&filename), bytes)
            .await
            .map_err(|e| format!("write climbs.png: {e}"))?;
        Some(filename)
    } else {
        None
    };

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let meta = json!({
        "created_at": created_at,
        "name": name,
        "total_km": output.total_km,
        "num_checkpoints": output.num_checkpoints,
        "num_climbs": output.num_climbs,
        "images": images_meta,
        "climb_stats_filename": climb_stats_filename,
    });

    tokio::fs::write(
        dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).expect("valid json"),
    )
    .await
    .map_err(|e| format!("write meta.json: {e}"))?;

    Ok(id)
}

async fn cleanup_shares() {
    let dir = share_dir();
    if !dir.exists() {
        return;
    }
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(_) => return,
    };
    let now = std::time::SystemTime::now();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let mut old = false;
        if let Ok(meta_str) = tokio::fs::read_to_string(path.join("meta.json")).await
            && let Ok(v) = serde_json::from_str::<Value>(&meta_str)
            && let Some(created_at) = v.get("created_at").and_then(|v| v.as_u64())
        {
            let created =
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(created_at);
            if now
                .duration_since(created)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                > SHARE_TTL_SECS
            {
                old = true;
            }
        }
        // Fallback: mtime.
        if !old
            && let Ok(md) = tokio::fs::metadata(&path).await
            && let Ok(modified) = md.modified()
            && now
                .duration_since(modified)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                > SHARE_TTL_SECS
        {
            old = true;
        }
        if old {
            let _ = tokio::fs::remove_dir_all(&path).await;
        }
    }
}

async fn share_page(headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Response {
    if !is_safe_id(&id) {
        return error_page("Invalid share id.").into_response();
    }
    let meta_path = share_dir().join(&id).join("meta.json");
    let meta_bytes = match tokio::fs::read(&meta_path).await {
        Ok(b) => b,
        Err(_) => {
            return error_page("This share link has expired or does not exist.").into_response();
        }
    };
    let meta: Value = match serde_json::from_slice(&meta_bytes) {
        Ok(v) => v,
        Err(e) => return error_page(&format!("Corrupt share metadata: {e}")).into_response(),
    };
    let base = public_base_url(&headers);
    Html(build_share_page(&id, &meta, &base)).into_response()
}

async fn share_file(AxumPath((id, file)): AxumPath<(String, String)>) -> Response {
    if !is_safe_id(&id) || !is_safe_filename(&file) {
        return (StatusCode::BAD_REQUEST, "Invalid path".to_string()).into_response();
    }
    let path = share_dir().join(&id).join(&file);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found".to_string()).into_response(),
    };
    let content_type = if file.ends_with(".gpx") {
        "application/gpx+xml"
    } else if file.ends_with(".png") {
        "image/png"
    } else if file.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    };
    // CORS so gpx.studio (and other browser-side viewers/unfurlers) can
    // fetch the file across origins.
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Length, Content-Type")
        .body(Body::from(bytes))
        .expect("valid response")
}

async fn share_file_options() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .header(header::ACCESS_CONTROL_MAX_AGE, "86400")
        .body(Body::empty())
        .expect("valid response")
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn build_share_page(id: &str, meta: &Value, base_url: &str) -> String {
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let total_km = meta.get("total_km").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let num_checkpoints = meta
        .get("num_checkpoints")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let num_climbs = meta.get("num_climbs").and_then(|v| v.as_u64()).unwrap_or(0);
    let created_at = meta
        .get("created_at")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let expires_at = created_at + SHARE_TTL_SECS;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs_left = expires_at.saturating_sub(now);
    let time_left_label = if secs_left == 0 {
        "shortly".to_string()
    } else if secs_left >= 24 * 3600 {
        let days = secs_left / (24 * 3600);
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else if secs_left >= 3600 {
        let h = secs_left / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else {
        let m = (secs_left / 60).max(1);
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    };
    let ttl_days = SHARE_TTL_SECS / (24 * 3600);
    let images = meta
        .get("images")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let share_abs = format!("{base_url}/share/{id}");
    let gpx_abs = format!("{base_url}/share/{id}/source.gpx");
    let og_image_abs = images
        .first()
        .and_then(|v| v.get("filename"))
        .and_then(|v| v.as_str())
        .map(|fname| format!("{base_url}/share/{id}/{fname}"))
        .unwrap_or_default();

    // gpx.studio expects ?files=<JSON-array of GPX URLs> and CORS-fetches each.
    let gpx_studio_url = format!(
        "https://gpx.studio/app?files={}",
        url_encode(&format!("[\"{}\"]", gpx_abs.replace('"', "\\\"")))
    );

    let og_title = if name.is_empty() {
        format!("GPX route — {total_km:.1} km")
    } else {
        format!("{name} — {total_km:.1} km")
    };
    let name_html = if name.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="route-name">{}</p>"#, html_escape(&name))
    };
    let name_json = serde_json::to_string(&name).unwrap_or_else(|_| "\"\"".to_string());
    let og_description = format!("{num_checkpoints} checkpoints · {num_climbs} climbs");
    let og_meta = if og_image_abs.is_empty() {
        String::new()
    } else {
        format!(
            r#"<meta property="og:type" content="website">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:url" content="{url}">
<meta property="og:image" content="{img}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{title}">
<meta name="twitter:description" content="{desc}">
<meta name="twitter:image" content="{img}">"#,
            title = html_escape(&og_title),
            desc = html_escape(&og_description),
            url = html_escape(&share_abs),
            img = html_escape(&og_image_abs),
        )
    };

    let mut images_html = String::new();
    for (i, img) in images.iter().enumerate() {
        let label = img.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let filename = img.get("filename").and_then(|v| v.as_str()).unwrap_or("");
        let safe_label = html_escape(label);
        let download_name = if images.len() == 1 {
            "profile.png".to_string()
        } else {
            format!("profile_{}.png", i + 1)
        };
        images_html.push_str(&format!(
            r#"
      <div class="image-card">
        <div class="image-label">{safe_label}</div>
        <img src="/share/{id}/{filename}" alt="{safe_label}">
        <a class="download-link" href="/share/{id}/{filename}" download="{download_name}">Download {safe_label}</a>
      </div>"#
        ));
    }
    if let Some(cs) = meta
        .get("climb_stats_filename")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        images_html.push_str(&format!(
            r#"
      <div class="image-card">
        <div class="image-label">Climb Statistics</div>
        <img src="/share/{id}/{cs}" alt="Climb Statistics">
        <a class="download-link" href="/share/{id}/{cs}" download="climb_stats.png">Download Climb Statistics</a>
      </div>"#
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Shared route &mdash; GPX to Graph</title>
{og_meta}
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:wght@400;600;700&family=Fraunces:opsz,wght@9..144,700;9..144,800&family=Space+Mono:wght@400;700&display=swap" rel="stylesheet">
<link rel="stylesheet" href="/static/recents.css">
<style>
  :root {{
    --paper: #f2e9d4; --paper-2: #e8dec0; --ink: #0e1424; --ink-soft: #2a2f3e;
    --muted: #79654a; --carmine: #b8242a; --teal: #14707a;
    --rule: rgba(14,20,36,.18);
    --f-display: 'Fraunces', Georgia, serif;
    --f-body: 'Bricolage Grotesque', system-ui, sans-serif;
    --f-mono: 'Space Mono', monospace;
  }}
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: var(--f-body);
    background: var(--paper);
    color: var(--ink);
    margin: 0;
    padding: 2rem 1rem;
  }}
  .grain {{
    pointer-events: none; position: fixed; inset: 0; z-index: 50;
    opacity: .22; mix-blend-mode: multiply;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='200' height='200'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0.055 0 0 0 0 0.078 0 0 0 0 0.141 0 0 0 0.5 0'/></filter><rect width='100%25' height='100%25' filter='url(%23n)'/></svg>");
  }}
  .container {{ max-width: 900px; margin: 0 auto; position: relative; z-index: 1; }}
  h1 {{
    font-family: var(--f-display);
    font-size: 1.75rem;
    font-weight: 800;
    margin: 0 0 0.25rem;
    font-variation-settings: "opsz" 72;
  }}
  .route-name {{
    margin: 0 0 1rem;
    font-size: 1.05rem;
    font-weight: 500;
    color: var(--muted);
    word-break: break-word;
  }}
  .back-link {{
    display: inline-block;
    margin-bottom: 1.5rem;
    color: var(--ink);
    text-decoration: none;
    font-weight: 600;
    font-size: 0.95rem;
    border-bottom: 1px solid var(--rule);
  }}
  .back-link:hover {{ border-bottom-color: var(--ink); }}
  .card {{
    background: var(--paper-2);
    border: 1px solid var(--ink);
    padding: 1.5rem 2rem;
    margin-bottom: 1.5rem;
  }}
  .summary {{ display: flex; gap: 2rem; flex-wrap: wrap; }}
  .stat {{ text-align: center; }}
  .stat-value {{
    font-family: var(--f-display);
    font-size: 1.5rem;
    font-weight: 700;
    color: var(--carmine);
    font-variation-settings: "opsz" 72;
  }}
  .stat-label {{
    font-family: var(--f-mono);
    font-size: 0.65rem; color: var(--muted);
    text-transform: uppercase; letter-spacing: 0.08em; margin-top: 0.2rem;
  }}
  .result-banner {{
    display: flex; flex-direction: column; gap: 0.6rem;
    border: 1px solid var(--ink); background: var(--paper-2);
  }}
  .result-banner .banner-title {{
    font-family: var(--f-mono);
    font-weight: 700; font-size: 0.75rem;
    text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--ink);
  }}
  .result-banner .banner-row {{ display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap; }}
  .result-banner input[type="text"] {{
    flex: 1 1 260px;
    min-width: 0;
    padding: 0.55rem 0.75rem;
    border: 1px solid var(--rule);
    font: inherit;
    background: var(--paper);
    color: var(--ink);
  }}
  .result-banner button {{
    padding: 0.55rem 0.9rem;
    background: var(--carmine);
    color: var(--paper);
    border: none;
    cursor: pointer;
    font-family: var(--f-mono);
    font-weight: 700;
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }}
  .result-banner button:hover {{ background: #961e22; }}
  .result-banner .ttl-note {{ font-size: 0.82rem; color: var(--muted); margin: 0; }}
  .result-banner a {{ color: var(--ink); font-weight: 600; text-decoration: none; font-size: 0.9rem; border-bottom: 1px solid var(--rule); }}
  .result-banner a:hover {{ border-bottom-color: var(--ink); }}
  .result-banner a.btn-link {{
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.55rem 0.9rem;
    background: var(--paper);
    color: var(--ink);
    border: 1px solid var(--ink);
    border-bottom: 1px solid var(--ink);
    font-weight: 600;
    font-size: 0.9rem;
    text-decoration: none;
    white-space: nowrap;
  }}
  .result-banner a.btn-link:hover {{ background: var(--paper-2); }}
  .image-card {{
    background: var(--paper-2);
    border: 1px solid var(--ink);
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }}
  .image-label {{
    font-family: var(--f-mono);
    font-weight: 700; font-size: 0.72rem;
    text-transform: uppercase; letter-spacing: 0.06em;
    margin-bottom: 1rem; color: var(--muted);
  }}
  .image-card img {{ max-width: 100%; height: auto; border: 1px solid var(--rule); }}
  .download-link {{
    display: inline-block;
    margin-top: 0.75rem;
    color: var(--ink);
    text-decoration: none;
    font-weight: 500;
    font-size: 0.9rem;
    border-bottom: 1px solid var(--rule);
  }}
  .download-link:hover {{ border-bottom-color: var(--ink); }}
</style>
<link rel="stylesheet" href="/static/themes.css">
<script>document.documentElement.dataset.theme=localStorage.getItem('gpx-theme')||'golden-hour'</script>
<script src="/static/theme-switcher.js" defer></script>
</head>
<body>
<div class="grain" aria-hidden="true"></div>
<aside id="recentsSidebar" class="recents-sidebar" aria-label="Recent routes" hidden>
  <div class="rx-header">
    <span class="rx-title">Recent routes</span>
    <button type="button" id="recentsClear" class="rx-clear" title="Clear all">×</button>
  </div>
  <ol id="recentsList" class="rx-list"></ol>
</aside>
<div class="container">
  <h1>Generated Profile</h1>
  {name_html}
  <a class="back-link" href="/">&larr; Generate another</a>

  <div class="card result-banner">
    <div class="banner-title">Share this result</div>
    <div class="banner-row">
      <input type="text" id="resultUrl" readonly>
      <button id="copyBtn" type="button">Copy link</button>
      <a class="btn-link" href="{gpx_studio_url}" target="_blank" rel="noopener">Open in gpx.studio &rarr;</a>
    </div>
    <p class="ttl-note">Link expires in about {time_left_label}. Results and the source GPX are kept for {ttl_days} days after creation.</p>
    <div><a href="/share/{id}/source.gpx" download="source.gpx">Download original GPX</a></div>
  </div>

  <div class="card">
    <div class="summary">
      <div class="stat">
        <div class="stat-value">{total_km:.1}</div>
        <div class="stat-label">Total KM</div>
      </div>
      <div class="stat">
        <div class="stat-value">{num_checkpoints}</div>
        <div class="stat-label">Checkpoints</div>
      </div>
      <div class="stat">
        <div class="stat-value">{num_climbs}</div>
        <div class="stat-label">Climbs</div>
      </div>
    </div>
  </div>

  {images_html}
</div>
<script>
  var urlInput = document.getElementById('resultUrl');
  urlInput.value = window.location.href;
  document.getElementById('copyBtn').addEventListener('click', async function () {{
    var btn = this;
    try {{
      await navigator.clipboard.writeText(urlInput.value);
    }} catch (e) {{
      urlInput.select();
      document.execCommand('copy');
    }}
    var prev = btn.textContent;
    btn.textContent = 'Copied!';
    setTimeout(function () {{ btn.textContent = prev; }}, 1500);
  }});
  window.__recentCurrent = {{
    id: {id_json},
    name: {name_json},
    total_km: {total_km},
    num_checkpoints: {num_checkpoints},
    num_climbs: {num_climbs},
    created_at: {created_at}
  }};
</script>
<script src="/static/recents.js" defer></script>
</body>
</html>"#,
        id = id,
        id_json = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string()),
        name_html = name_html,
        name_json = name_json,
        total_km = total_km,
        num_checkpoints = num_checkpoints,
        num_climbs = num_climbs,
        created_at = created_at,
        time_left_label = time_left_label,
        ttl_days = ttl_days,
        images_html = images_html,
        og_meta = og_meta,
        gpx_studio_url = gpx_studio_url,
    )
}

// ---------------------------------------------------------------------------
// FIT parsing – keeps data in native format until merge time
// ---------------------------------------------------------------------------

fn parse_fit_trkpts(data: &[u8]) -> Result<Vec<TrackPoint>, String> {
    use fitparser::profile::MesgNum;
    use fitparser::Value;
    use std::io::Cursor;

    const SC_TO_DEG: f64 = 180.0 / 2_147_483_648.0;

    let records =
        fitparser::from_reader(&mut Cursor::new(data)).map_err(|e| format!("FIT parse: {e}"))?;

    let mut out = Vec::new();
    for rec in &records {
        if rec.kind() != MesgNum::Record {
            continue;
        }

        let mut lat: Option<f64> = None;
        let mut lon: Option<f64> = None;
        let mut pt = TrackPoint::default();

        for field in rec.fields() {
            match field.name() {
                "position_lat" => {
                    if let Value::SInt32(v) = field.value() {
                        lat = Some(*v as f64 * SC_TO_DEG);
                    }
                }
                "position_long" => {
                    if let Value::SInt32(v) = field.value() {
                        lon = Some(*v as f64 * SC_TO_DEG);
                    }
                }
                "enhanced_altitude" | "altitude" => {
                    if pt.ele.is_none() {
                        pt.ele = match field.value() {
                            Value::Float64(v) => Some(*v),
                            Value::Float32(v) => Some(*v as f64),
                            Value::UInt16(v) => Some(*v as f64 / 5.0 - 500.0),
                            _ => None,
                        };
                    }
                }
                "timestamp" => {
                    if let Value::Timestamp(ts) = field.value() {
                        pt.time = Some(ts.to_utc().timestamp() as f64);
                    }
                }
                "heart_rate" => {
                    if let Value::UInt8(v) = field.value() {
                        pt.hr = Some(*v as f64);
                    }
                }
                "cadence" => {
                    if let Value::UInt8(v) = field.value() {
                        pt.cad = Some(*v as f64);
                    }
                }
                "power" => {
                    if let Value::UInt16(v) = field.value() {
                        pt.power = Some(*v as f64);
                    }
                }
                "temperature" => {
                    if let Value::SInt8(v) = field.value() {
                        pt.temp = Some(*v as f64);
                    }
                }
                _ => {}
            }
        }

        let (Some(la), Some(lo)) = (lat, lon) else {
            continue;
        };
        pt.lat = la;
        pt.lon = lo;
        out.push(pt);
    }
    Ok(out)
}

fn trackpoints_to_gpx_bytes(pts: &[TrackPoint]) -> Vec<u8> {
    use std::fmt::Write;

    let mut xml = String::with_capacity(pts.len() * 200);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <gpx version=\"1.1\" creator=\"gpx_to_graph\"\n  \
        xmlns=\"http://www.topografix.com/GPX/1/1\"\n  \
        xmlns:gpxtpx=\"http://www.garmin.com/xmlschemas/TrackPointExtension/v1\"\n  \
        xmlns:gpxpx=\"http://www.garmin.com/xmlschemas/PowerExtension/v1\">\n\
        <trk><trkseg>\n");

    for p in pts {
        let _ = write!(xml, "<trkpt lat=\"{:.7}\" lon=\"{:.7}\">", p.lat, p.lon);
        if let Some(e) = p.ele {
            let _ = write!(xml, "<ele>{e:.1}</ele>");
        }
        if let Some(epoch) = p.time {
            if let Some(t) = chrono::DateTime::from_timestamp(epoch as i64, 0) {
                let _ = write!(xml, "<time>{}</time>", t.format("%Y-%m-%dT%H:%M:%SZ"));
            }
        }

        let has_ext = p.hr.is_some() || p.cad.is_some() || p.power.is_some() || p.temp.is_some();
        if has_ext {
            xml.push_str("<extensions>");
            if p.hr.is_some() || p.cad.is_some() || p.temp.is_some() {
                xml.push_str("<gpxtpx:TrackPointExtension>");
                if let Some(v) = p.hr {
                    let _ = write!(xml, "<gpxtpx:hr>{}</gpxtpx:hr>", v as u16);
                }
                if let Some(v) = p.cad {
                    let _ = write!(xml, "<gpxtpx:cad>{}</gpxtpx:cad>", v as u16);
                }
                if let Some(v) = p.temp {
                    let _ = write!(xml, "<gpxtpx:atemp>{}</gpxtpx:atemp>", v as i16);
                }
                xml.push_str("</gpxtpx:TrackPointExtension>");
            }
            if let Some(v) = p.power {
                let _ = write!(xml, "<gpxpx:PowerExtension><gpxpx:PowerInWatts>{}</gpxpx:PowerInWatts></gpxpx:PowerExtension>", v as u16);
            }
            xml.push_str("</extensions>");
        }

        xml.push_str("</trkpt>\n");
    }

    xml.push_str("</trkseg></trk></gpx>\n");
    xml.into_bytes()
}

// ---------------------------------------------------------------------------
// Douglas-Peucker simplification
// ---------------------------------------------------------------------------

fn perp_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    let t = ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2;
    let t = t.clamp(0.0, 1.0);
    let px = a.0 + t * dx;
    let py = a.1 + t * dy;
    ((p.0 - px).powi(2) + (p.1 - py).powi(2)).sqrt()
}

fn douglas_peucker(coords: &[(f64, f64)], tolerance: f64) -> Vec<bool> {
    let n = coords.len();
    let mut keep = vec![false; n];
    if n <= 2 {
        for k in &mut keep {
            *k = true;
        }
        return keep;
    }
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((start, end)) = stack.pop() {
        let mut max_dist = 0.0_f64;
        let mut max_idx = start;
        for i in (start + 1)..end {
            let d = perp_dist(coords[i], coords[start], coords[end]);
            if d > max_dist {
                max_dist = d;
                max_idx = i;
            }
        }
        if max_dist > tolerance {
            keep[max_idx] = true;
            stack.push((start, max_idx));
            stack.push((max_idx, end));
        }
    }
    keep
}

fn dp_count(coords: &[(f64, f64)], tolerance: f64) -> usize {
    let n = coords.len();
    if n <= 2 {
        return n;
    }
    let mut count = 2usize;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((start, end)) = stack.pop() {
        let mut max_dist = 0.0_f64;
        let mut max_idx = start;
        for i in (start + 1)..end {
            let d = perp_dist(coords[i], coords[start], coords[end]);
            if d > max_dist {
                max_dist = d;
                max_idx = i;
            }
        }
        if max_dist > tolerance {
            count += 1;
            stack.push((start, max_idx));
            stack.push((max_idx, end));
        }
    }
    count
}

fn find_tolerance_for_pct(coords: &[(f64, f64)], target_pct: u32) -> f64 {
    if target_pct == 0 || coords.len() <= 2 {
        return 0.0;
    }
    let total = coords.len();
    let target_keep = (total as f64 * (1.0 - target_pct as f64 / 100.0))
        .round()
        .max(2.0) as usize;
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        let kept = dp_count(coords, mid);
        if kept == target_keep {
            return mid;
        }
        if kept < target_keep {
            hi = mid;
        } else {
            lo = mid;
        }
        if (hi - lo) / (hi + 1e-15) < 1e-6 {
            break;
        }
    }
    lo
}

fn simplify_trackpoints(
    pts: &[TrackPoint],
    target_pct: u32,
    strip: &std::collections::HashSet<String>,
) -> (Vec<TrackPoint>, usize, usize) {
    let coords: Vec<(f64, f64)> = pts.iter().map(|p| (p.lat, p.lon)).collect();
    let before = coords.len();
    let tol = find_tolerance_for_pct(&coords, target_pct);
    let keep = if tol == 0.0 {
        vec![true; before]
    } else {
        douglas_peucker(&coords, tol)
    };
    let out: Vec<TrackPoint> = pts
        .iter()
        .zip(keep.iter())
        .filter(|&(_, &k)| k)
        .map(|(p, _)| {
            let mut pt = p.clone();
            if strip.contains("time") {
                pt.time = None;
            }
            if strip.contains("ele") {
                pt.ele = None;
            }
            if strip.contains("extensions") {
                pt.hr = None;
                pt.cad = None;
                pt.power = None;
                pt.temp = None;
            } else {
                if strip.contains("hr") {
                    pt.hr = None;
                }
                if strip.contains("cad") {
                    pt.cad = None;
                }
                if strip.contains("power") {
                    pt.power = None;
                }
                if strip.contains("atemp") {
                    pt.temp = None;
                }
            }
            pt
        })
        .collect();
    let after = out.len();
    (out, before, after)
}

async fn simplify_handler(mut multipart: Multipart) -> Response {
    let mut file_data: Option<(Option<String>, Vec<u8>)> = None;
    let mut pct: u32 = 0;
    let mut strip: std::collections::HashSet<String> = std::collections::HashSet::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                let filename = field.file_name().map(String::from);
                if let Ok(data) = field.bytes().await {
                    file_data = Some((filename, data.to_vec()));
                }
            }
            "pct" => {
                if let Ok(text) = field.text().await {
                    pct = text.trim().parse().unwrap_or(0).min(99);
                }
            }
            "strip" => {
                if let Ok(text) = field.text().await {
                    for tag in text.split(',') {
                        let t = tag.trim();
                        if !t.is_empty() {
                            strip.insert(t.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let Some((filename, data)) = file_data else {
        return (StatusCode::BAD_REQUEST, "No file uploaded").into_response();
    };

    let pts = if is_fit_file(filename.as_deref(), &data) {
        match parse_fit_trkpts(&data) {
            Ok(p) => p,
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    } else {
        parse_all_trkpts(&data)
    };

    if pts.is_empty() {
        return (StatusCode::BAD_REQUEST, "No track points found").into_response();
    }

    let (simplified, before, after) = simplify_trackpoints(&pts, pct, &strip);
    let gpx_bytes = trackpoints_to_gpx_bytes(&simplified);
    let coords: Vec<[f64; 2]> = simplified.iter().map(|p| [p.lat, p.lon]).collect();

    let resp = serde_json::json!({
        "xml": String::from_utf8_lossy(&gpx_bytes),
        "before": before,
        "after": after,
        "coords": coords,
        "size": gpx_bytes.len(),
    });

    Json(resp).into_response()
}

fn is_fit_file(filename: Option<&str>, data: &[u8]) -> bool {
    if let Some(name) = filename {
        if name.to_ascii_lowercase().ends_with(".fit") {
            return true;
        }
    }
    data.len() >= 14 && &data[8..12] == b".FIT"
}

// ---------------------------------------------------------------------------
// Byte-level GPX merge
//
// The `gpx` crate (0.10) drops <extensions> during parse (see its
// `parser/extensions.rs`: "TODO: extensions are not implemented"), which would
// lose heart-rate, cadence, power, temperature and any other per-point sensor
// data carried by the Garmin TrackPointExtension schema. To keep that data
// intact we merge at the raw XML level: we never re-serialise trkpt content,
// we only splice whole <trkseg>…</trkseg> blocks from the other files into the
// base file's first <trk>. This preserves every child element verbatim,
// including <extensions> and any custom namespaces.
// ---------------------------------------------------------------------------

fn find_after(data: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > data.len() {
        return None;
    }
    data[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

fn find_trkseg_open(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    loop {
        let abs = find_after(data, b"<trkseg", i)?;
        let after = data.get(abs + b"<trkseg".len()).copied();
        match after {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/') => {
                return Some(abs)
            }
            _ => i = abs + b"<trkseg".len(),
        }
    }
}

fn extract_trksegs(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_trkseg_open(data, pos) {
        let Some(gt) = find_after(data, b">", start) else {
            break;
        };
        if gt > 0 && data[gt - 1] == b'/' {
            out.push(data[start..=gt].to_vec());
            pos = gt + 1;
            continue;
        }
        let Some(close) = find_after(data, b"</trkseg>", gt) else {
            break;
        };
        let close_end = close + b"</trkseg>".len();
        out.push(data[start..close_end].to_vec());
        pos = close_end;
    }
    out
}

fn first_trkpt_time(data: &[u8]) -> Option<String> {
    let tp = find_after(data, b"<trkpt", 0)?;
    let time_tag = find_after(data, b"<time>", tp)?;
    let value_start = time_tag + b"<time>".len();
    let time_end = find_after(data, b"</time>", value_start)?;
    std::str::from_utf8(&data[value_start..time_end])
        .ok()
        .map(|s| s.trim().to_string())
}

fn escape_xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn rewrite_gpx_creator(data: &[u8], new_creator: &str) -> Vec<u8> {
    let Some(start) = find_after(data, b"<gpx", 0) else {
        return data.to_vec();
    };
    let Some(end) = find_after(data, b">", start) else {
        return data.to_vec();
    };

    if let Some(rel_c) = data[start..=end]
        .windows(b"creator=".len())
        .position(|w| w == b"creator=")
    {
        let after_eq = start + rel_c + b"creator=".len();
        if after_eq < data.len() {
            let quote = data[after_eq];
            if quote == b'"' || quote == b'\'' {
                let value_start = after_eq + 1;
                if let Some(rel_end) = data[value_start..=end].iter().position(|&c| c == quote) {
                    let value_end = value_start + rel_end;
                    let escaped = escape_xml_attr(new_creator);
                    let mut out = Vec::with_capacity(data.len() + escaped.len());
                    out.extend_from_slice(&data[..value_start]);
                    out.extend_from_slice(escaped.as_bytes());
                    out.extend_from_slice(&data[value_end..]);
                    return out;
                }
            }
        }
    }

    let insert_at = if end > 0 && data[end - 1] == b'/' {
        end - 1
    } else {
        end
    };
    let escaped = escape_xml_attr(new_creator);
    let insertion = format!(" creator=\"{escaped}\"");
    let mut out = Vec::with_capacity(data.len() + insertion.len());
    out.extend_from_slice(&data[..insert_at]);
    out.extend_from_slice(insertion.as_bytes());
    out.extend_from_slice(&data[insert_at..]);
    out
}

fn splice_trksegs_before_first_close_trk(base: &[u8], extras: &[Vec<u8>]) -> Vec<u8> {
    if extras.is_empty() {
        return base.to_vec();
    }
    let Some(close_pos) = find_after(base, b"</trk>", 0) else {
        return base.to_vec();
    };
    let extras_total: usize = extras.iter().map(|e| e.len() + 1).sum();
    let mut out = Vec::with_capacity(base.len() + extras_total);
    out.extend_from_slice(&base[..close_pos]);
    for seg in extras {
        out.extend_from_slice(seg);
        out.push(b'\n');
    }
    out.extend_from_slice(&base[close_pos..]);
    out
}

fn ensure_gpx_namespaces(data: Vec<u8>) -> Vec<u8> {
    let Some(gpx_start) = find_after(&data, b"<gpx", 0) else {
        return data;
    };
    let Some(gpx_end) = find_after(&data, b">", gpx_start) else {
        return data;
    };
    let header = &data[gpx_start..=gpx_end];

    let ns: &[(&[u8], &str)] = &[
        (
            b"gpxtpx:" as &[u8],
            " xmlns:gpxtpx=\"http://www.garmin.com/xmlschemas/TrackPointExtension/v1\"",
        ),
        (
            b"gpxpx:",
            " xmlns:gpxpx=\"http://www.garmin.com/xmlschemas/PowerExtension/v1\"",
        ),
    ];

    let mut insertions = String::new();
    for (prefix, decl) in ns {
        let xmlns_key = format!("xmlns:{}=", std::str::from_utf8(&prefix[..prefix.len() - 1]).unwrap_or(""));
        if data.windows(prefix.len()).any(|w| w == *prefix)
            && !header.windows(xmlns_key.len()).any(|w| w == xmlns_key.as_bytes())
        {
            insertions.push_str(decl);
        }
    }

    if insertions.is_empty() {
        return data;
    }

    let insert_at = if gpx_end > 0 && data[gpx_end - 1] == b'/' {
        gpx_end - 1
    } else {
        gpx_end
    };
    let mut out = Vec::with_capacity(data.len() + insertions.len());
    out.extend_from_slice(&data[..insert_at]);
    out.extend_from_slice(insertions.as_bytes());
    out.extend_from_slice(&data[insert_at..]);
    out
}

fn merge_gpx_preserving_extensions(
    files: Vec<Vec<u8>>,
    creator: Option<String>,
) -> Result<Vec<u8>, String> {
    if files.is_empty() {
        return Err("no files provided".to_string());
    }

    let mut indexed: Vec<(Option<String>, Vec<u8>)> = files
        .into_iter()
        .map(|f| (first_trkpt_time(&f), f))
        .collect();
    indexed.sort_by(|a, b| a.0.cmp(&b.0));

    let mut iter = indexed.into_iter();
    let (_, base) = iter.next().expect("non-empty after check");
    let extras: Vec<Vec<u8>> = iter.flat_map(|(_, f)| extract_trksegs(&f)).collect();

    let base = match creator.as_deref() {
        Some(c) => rewrite_gpx_creator(&base, c),
        None => base,
    };

    let merged = splice_trksegs_before_first_close_trk(&base, &extras);
    Ok(ensure_gpx_namespaces(merged))
}

// ---------------------------------------------------------------------------
// Post-merge stats: distance, elevation, heart rate, cadence, power, temp.
// Parsed straight from the merged XML (byte-level, same assumptions as the
// merge step) so we never depend on `gpx` crate for <extensions>.
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct TrackPoint {
    lat: f64,
    lon: f64,
    ele: Option<f64>,
    time: Option<f64>,
    hr: Option<f64>,
    cad: Option<f64>,
    power: Option<f64>,
    temp: Option<f64>,
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_iso8601_epoch(s: &str) -> Option<f64> {
    let b = s.trim().as_bytes();
    if b.len() < 19 {
        return None;
    }
    let year: i64 = std::str::from_utf8(&b[0..4]).ok()?.parse().ok()?;
    let month: i64 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
    let day: i64 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
    let hour: i64 = std::str::from_utf8(&b[11..13]).ok()?.parse().ok()?;
    let minute: i64 = std::str::from_utf8(&b[14..16]).ok()?.parse().ok()?;
    let sec: i64 = std::str::from_utf8(&b[17..19]).ok()?.parse().ok()?;
    let mut i = 19;
    let frac = if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac_str = std::str::from_utf8(&b[start..i]).ok()?;
        format!("0.{frac_str}").parse::<f64>().unwrap_or(0.0)
    } else {
        0.0
    };
    let tz_offset_sec: i64 = if i < b.len() {
        match b[i] {
            b'Z' | b'z' => 0,
            b'+' | b'-' => {
                let sign: i64 = if b[i] == b'+' { 1 } else { -1 };
                i += 1;
                if b.len() < i + 2 {
                    return None;
                }
                let th: i64 = std::str::from_utf8(&b[i..i + 2]).ok()?.parse().ok()?;
                i += 2;
                if b.get(i) == Some(&b':') {
                    i += 1;
                }
                let tm: i64 = if b.len() >= i + 2 {
                    std::str::from_utf8(&b[i..i + 2]).ok()?.parse().ok()?
                } else {
                    0
                };
                sign * (th * 3600 + tm * 60)
            }
            _ => 0,
        }
    } else {
        0
    };
    let days = days_from_civil(year, month, day);
    let secs = days * 86400 + hour * 3600 + minute * 60 + sec;
    Some(secs as f64 + frac - tz_offset_sec as f64)
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().atan2((1.0 - a).sqrt())
}

fn parse_attr_f64(attrs: &[u8], name: &[u8]) -> Option<f64> {
    let mut i = 0;
    while let Some(p) = find_after(attrs, name, i) {
        let preceded_ok = p == 0 || matches!(attrs[p - 1], b' ' | b'\t' | b'\n' | b'\r');
        if !preceded_ok {
            i = p + name.len();
            continue;
        }
        let mut j = p + name.len();
        while j < attrs.len() && matches!(attrs[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= attrs.len() || attrs[j] != b'=' {
            i = p + name.len();
            continue;
        }
        j += 1;
        while j < attrs.len() && matches!(attrs[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= attrs.len() {
            return None;
        }
        let quote = attrs[j];
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        j += 1;
        let start = j;
        while j < attrs.len() && attrs[j] != quote {
            j += 1;
        }
        if j >= attrs.len() {
            return None;
        }
        return std::str::from_utf8(&attrs[start..j])
            .ok()?
            .trim()
            .parse()
            .ok();
    }
    None
}

fn extract_tag_text(body: &[u8], tag: &[u8]) -> Option<String> {
    let mut i = 0;
    loop {
        let open = find_after(body, b"<", i)?;
        let name_start = open + 1;
        if name_start + tag.len() > body.len() {
            return None;
        }
        if &body[name_start..name_start + tag.len()] != tag {
            i = open + 1;
            continue;
        }
        let after_name = name_start + tag.len();
        let next = body.get(after_name).copied();
        if !matches!(
            next,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')
        ) {
            i = open + 1;
            continue;
        }
        let gt = find_after(body, b">", after_name)?;
        if gt > 0 && body[gt - 1] == b'/' {
            i = gt + 1;
            continue;
        }
        let value_start = gt + 1;
        let close_needle: Vec<u8> = [b"</", tag, b">"].concat();
        let close = find_after(body, &close_needle, value_start)?;
        return std::str::from_utf8(&body[value_start..close])
            .ok()
            .map(|s| s.trim().to_string());
    }
}

fn extract_tag_value_f64(body: &[u8], tag: &[u8]) -> Option<f64> {
    extract_tag_text(body, tag)?.parse().ok()
}

fn extract_ns_or_local_f64(body: &[u8], local: &[u8]) -> Option<f64> {
    for prefix in [b"gpxtpx:".as_ref(), b"gpxdata:".as_ref(), b"".as_ref()] {
        let full: Vec<u8> = [prefix, local].concat();
        if let Some(v) = extract_tag_value_f64(body, &full) {
            return Some(v);
        }
    }
    None
}

fn parse_all_trkpts(data: &[u8]) -> Vec<TrackPoint> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_after(data, b"<trkpt", pos) {
        let after_tag = start + b"<trkpt".len();
        let next = data.get(after_tag).copied();
        if !matches!(
            next,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')
        ) {
            pos = after_tag;
            continue;
        }
        let Some(gt) = find_after(data, b">", after_tag) else {
            break;
        };
        let attrs = &data[after_tag..gt];
        let (Some(lat), Some(lon)) = (parse_attr_f64(attrs, b"lat"), parse_attr_f64(attrs, b"lon"))
        else {
            pos = gt + 1;
            continue;
        };

        let self_closing = gt > 0 && data[gt - 1] == b'/';
        let (body, advance) = if self_closing {
            (&data[0..0], gt + 1)
        } else {
            match find_after(data, b"</trkpt>", gt) {
                Some(e) => (&data[gt + 1..e], e + b"</trkpt>".len()),
                None => break,
            }
        };

        out.push(TrackPoint {
            lat,
            lon,
            ele: extract_tag_value_f64(body, b"ele"),
            time: extract_tag_text(body, b"time").and_then(|s| parse_iso8601_epoch(&s)),
            hr: extract_ns_or_local_f64(body, b"hr"),
            cad: extract_ns_or_local_f64(body, b"cad"),
            power: extract_ns_or_local_f64(body, b"power")
                .or_else(|| extract_tag_value_f64(body, b"gpxpx:PowerInWatts"))
                .or_else(|| extract_tag_value_f64(body, b"PowerInWatts")),
            temp: extract_ns_or_local_f64(body, b"atemp"),
        });

        pos = advance;
    }
    out
}

fn stats_from_points(pts: &[TrackPoint]) -> Value {
    let mut kms: Vec<f64> = Vec::with_capacity(pts.len());
    let mut total_km = 0.0_f64;
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            total_km += haversine_km(pts[i - 1].lat, pts[i - 1].lon, p.lat, p.lon);
        }
        kms.push(total_km);
    }

    // Time: separate "total span" (first → last timestamp) from "moving time"
    // (only intervals where speed ≥ threshold). Anything longer than 4 h
    // between two consecutive points is treated as a dead gap and ignored
    // for moving-time purposes.
    const MOVING_SPEED_KMH: f64 = 1.5;
    // Any interval longer than this between two consecutive points is treated
    // as a stop regardless of the implied speed — covers smart-pause gaps,
    // inter-file boundaries after merge, etc.
    const MAX_GAP_S: f64 = 300.0;
    let first_time = pts.iter().find_map(|p| p.time);
    let last_time = pts.iter().rev().find_map(|p| p.time);
    let total_time_s = match (first_time, last_time) {
        (Some(a), Some(b)) if b > a => b - a,
        _ => 0.0,
    };
    let mut moving_time_s = 0.0_f64;
    let mut moving_distance_km = 0.0_f64;
    for w in pts.windows(2) {
        if let (Some(t0), Some(t1)) = (w[0].time, w[1].time) {
            let dt = t1 - t0;
            if dt > 0.0 && dt < MAX_GAP_S {
                let dkm = haversine_km(w[0].lat, w[0].lon, w[1].lat, w[1].lon);
                let speed_kmh = dkm / (dt / 3600.0);
                if speed_kmh >= MOVING_SPEED_KMH {
                    moving_time_s += dt;
                    moving_distance_km += dkm;
                }
            }
        }
    }
    let avg_speed_kmh = if total_time_s > 0.0 {
        total_km / (total_time_s / 3600.0)
    } else {
        0.0
    };
    let avg_moving_speed_kmh = if moving_time_s > 0.0 {
        moving_distance_km / (moving_time_s / 3600.0)
    } else {
        0.0
    };

    let mut ele: Vec<(f64, f64)> = Vec::new();
    let mut hr: Vec<(f64, f64)> = Vec::new();
    let mut cad: Vec<(f64, f64)> = Vec::new();
    let mut pow: Vec<(f64, f64)> = Vec::new();
    let mut tmp: Vec<(f64, f64)> = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        let km = kms[i];
        if let Some(v) = p.ele {
            ele.push((km, v));
        }
        if let Some(v) = p.hr {
            hr.push((km, v));
        }
        if let Some(v) = p.cad {
            cad.push((km, v));
        }
        if let Some(v) = p.power {
            pow.push((km, v));
        }
        if let Some(v) = p.temp {
            tmp.push((km, v));
        }
    }

    fn summary(s: &[(f64, f64)]) -> Option<Value> {
        if s.is_empty() {
            return None;
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        for &(_, v) in s {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            sum += v;
        }
        Some(json!({
            "avg": sum / s.len() as f64,
            "min": min,
            "max": max,
            "samples": s.len(),
        }))
    }

    fn series(s: &[(f64, f64)]) -> Value {
        let km: Vec<f64> = s.iter().map(|&(k, _)| k).collect();
        let value: Vec<f64> = s.iter().map(|&(_, v)| v).collect();
        json!({ "km": km, "value": value })
    }

    let mut ele_gain = 0.0_f64;
    let mut ele_loss = 0.0_f64;
    for w in ele.windows(2) {
        let d = w[1].1 - w[0].1;
        if d > 0.0 {
            ele_gain += d;
        } else {
            ele_loss -= d;
        }
    }
    let elevation = summary(&ele).map(|s| {
        let mut m = s.as_object().cloned().unwrap_or_default();
        m.insert("gain".to_string(), json!(ele_gain));
        m.insert("loss".to_string(), json!(ele_loss));
        Value::Object(m)
    });

    json!({
        "total_km": total_km,
        "point_count": pts.len(),
        "total_time_s": total_time_s,
        "moving_time_s": moving_time_s,
        "idle_time_s": (total_time_s - moving_time_s).max(0.0),
        "avg_speed_kmh": avg_speed_kmh,
        "avg_moving_speed_kmh": avg_moving_speed_kmh,
        "elevation": elevation,
        "hr": summary(&hr),
        "cadence": summary(&cad),
        "power": summary(&pow),
        "temperature": summary(&tmp),
        "series": {
            "elevation": series(&ele),
            "hr": series(&hr),
            "cadence": series(&cad),
            "power": series(&pow),
            "temperature": series(&tmp),
        }
    })
}

async fn merge_handler(mut multipart: Multipart) -> Response {
    let mut files: Vec<(Option<String>, Vec<u8>)> = Vec::new();
    let mut creator: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "files" => {
                let filename = field.file_name().map(String::from);
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    const MAX_FILE: usize = 100 * 1024 * 1024;
                    if data.len() > MAX_FILE {
                        let label = filename.as_deref().unwrap_or("(unnamed)");
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("File '{}' is too large ({:.1} MB, max 100 MB).",
                                    label, data.len() as f64 / (1024.0 * 1024.0)),
                        )
                            .into_response();
                    }
                    files.push((filename, data.to_vec()));
                }
            }
            "creator" => {
                if let Ok(text) = field.text().await {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        creator = Some(trimmed);
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    if !(2..=5).contains(&files.len()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Expected 2 to 5 GPX files, got {}.", files.len()),
        )
            .into_response();
    }

    let creator_for_task = creator.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Value, Value), String> {
        let per_file: Vec<Value> = files
            .iter()
            .map(|(name, data)| {
                let pts = parse_all_trkpts(data);
                json!({
                    "name": name.clone().unwrap_or_else(|| "(unnamed)".to_string()),
                    "stats": stats_from_points(&pts),
                })
            })
            .collect();
        let raw_files: Vec<Vec<u8>> = files.into_iter().map(|(_, d)| d).collect();
        let merged = merge_gpx_preserving_extensions(raw_files, creator_for_task)?;
        let merged_pts = parse_all_trkpts(&merged);
        let stats = stats_from_points(&merged_pts);
        Ok((merged, stats, Value::Array(per_file)))
    })
    .await;

    match result {
        Ok(Ok((bytes, stats, per_file))) => {
            let gpx_str = match std::str::from_utf8(&bytes) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "merged GPX contained non-UTF-8 bytes".to_string(),
                    )
                        .into_response();
                }
            };
            let payload = json!({
                "gpx": gpx_str,
                "stats": stats,
                "per_file": per_file,
            });
            let body = serde_json::to_vec(&payload).expect("valid json");
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("valid response")
        }
        Ok(Err(msg)) => (StatusCode::BAD_REQUEST, msg).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task failed: {e}"),
        )
            .into_response(),
    }
}

async fn merge_upload_handler(
    State(sessions): State<MergeSessions>,
    mut multipart: Multipart,
) -> Response {
    let mut session_id: Option<String> = None;
    let mut file_data: Option<(Option<String>, Vec<u8>)> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "session" => {
                if let Ok(text) = field.text().await {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        session_id = Some(trimmed);
                    }
                }
            }
            "file" => {
                let filename = field.file_name().map(String::from);
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    const MAX_FILE: usize = 100 * 1024 * 1024;
                    if data.len() > MAX_FILE {
                        let label = filename.as_deref().unwrap_or("(unnamed)");
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("File '{}' is too large ({:.1} MB, max 100 MB).",
                                    label, data.len() as f64 / (1024.0 * 1024.0)),
                        )
                            .into_response();
                    }
                    file_data = Some((filename, data.to_vec()));
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let (filename, data) = match file_data {
        Some(f) => f,
        None => return (StatusCode::BAD_REQUEST, "No file provided.".to_string()).into_response(),
    };

    let mut map = sessions.lock().await;

    // Purge sessions older than 10 minutes
    map.retain(|_, s| s.created.elapsed().as_secs() < 600);

    let sid = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let session = map.entry(sid.clone()).or_insert_with(|| MergeSession {
        files: Vec::new(),
        created: Instant::now(),
    });

    if session.files.len() >= 5 {
        return (StatusCode::BAD_REQUEST, "Maximum 5 files per merge session.".to_string())
            .into_response();
    }

    session.files.push((filename, data));
    let count = session.files.len();
    drop(map);

    Json(json!({ "session": sid, "count": count })).into_response()
}

async fn merge_run_handler(
    State(sessions): State<MergeSessions>,
    Json(body): Json<Value>,
) -> Response {
    let sid = match body.get("session").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing session id.".to_string()).into_response(),
    };
    let creator = body
        .get("creator")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let files = {
        let mut map = sessions.lock().await;
        match map.remove(&sid) {
            Some(s) => s.files,
            None => {
                return (StatusCode::BAD_REQUEST, "Unknown or expired session.".to_string())
                    .into_response()
            }
        }
    };

    if !(2..=5).contains(&files.len()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Expected 2 to 5 files, got {}.", files.len()),
        )
            .into_response();
    }

    let creator_for_task = creator;
    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Value, Value), String> {
        let mut per_file: Vec<Value> = Vec::new();
        let mut gpx_files: Vec<Vec<u8>> = Vec::new();

        for (name, data) in &files {
            let label = name.clone().unwrap_or_else(|| "(unnamed)".to_string());
            if is_fit_file(name.as_deref(), data) {
                let pts = parse_fit_trkpts(data)?;
                per_file.push(json!({ "name": label, "stats": stats_from_points(&pts) }));
                gpx_files.push(trackpoints_to_gpx_bytes(&pts));
            } else {
                let pts = parse_all_trkpts(data);
                per_file.push(json!({ "name": label, "stats": stats_from_points(&pts) }));
                gpx_files.push(data.clone());
            }
        }

        let merged = merge_gpx_preserving_extensions(gpx_files, creator_for_task)?;
        let merged_pts = parse_all_trkpts(&merged);
        let stats = stats_from_points(&merged_pts);
        Ok((merged, stats, Value::Array(per_file)))
    })
    .await;

    match result {
        Ok(Ok((bytes, stats, per_file))) => {
            let gpx_str = match std::str::from_utf8(&bytes) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "merged GPX contained non-UTF-8 bytes".to_string(),
                    )
                        .into_response();
                }
            };
            let payload = json!({
                "gpx": gpx_str,
                "stats": stats,
                "per_file": per_file,
            });
            let body = serde_json::to_vec(&payload).expect("valid json");
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("valid response")
        }
        Ok(Err(msg)) => (StatusCode::BAD_REQUEST, msg).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task failed: {e}"),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    // --- Meteo service ---
    let meteo_db_path = std::env::var("METEO_DB_PATH").unwrap_or_else(|_| "data/meteo.db".into());
    if let Some(parent) = std::path::Path::new(&meteo_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let meteo_cache = std::sync::Arc::new(
        gpx_to_graph::meteo::weather::WeatherCache::open(&meteo_db_path)
            .expect("failed to open meteo cache"),
    );

    // --- Ravito service ---
    let ravito_db_path = std::env::var("RAVITO_DB_PATH").unwrap_or_else(|_| "data/ravito.db".into());
    if let Some(parent) = std::path::Path::new(&ravito_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let overpass_url = std::env::var("OVERPASS_URL")
        .unwrap_or_else(|_| "https://overpass-api.de/api/interpreter".into());
    let ravito_cache = std::sync::Arc::new(
        gpx_to_graph::ravito::overpass::OverpassCache::open(&ravito_db_path, overpass_url)
            .expect("failed to open ravito cache"),
    );

    // --- Trace service ---
    let trace_db_path = std::env::var("TRACE_DB_PATH").unwrap_or_else(|_| "data/trace.db".into());
    if let Some(parent) = std::path::Path::new(&trace_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let trace_db = gpx_to_graph::trace::db::Db::open(&trace_db_path)
        .expect("failed to open trace db");
    trace_db.migrate().expect("failed to migrate trace db");
    let trace_state: gpx_to_graph::trace::SharedState = std::sync::Arc::new(
        gpx_to_graph::trace::AppState {
            db: trace_db,
            channels: gpx_to_graph::trace::session::Channels::new(),
        },
    );

    // Purge expired trace sessions every 10 minutes.
    let purge_state = trace_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            interval.tick().await;
            if let Err(e) = purge_state.db.purge_expired() {
                tracing::warn!("trace purge error: {e}");
            }
        }
    });

    // --- Col service ---
    let col_db_path = std::env::var("COL_DB_PATH").unwrap_or_else(|_| "data/col.db".into());
    if let Some(parent) = std::path::Path::new(&col_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let col_db = gpx_to_graph::col::db::Db::open(&col_db_path)
        .expect("failed to open col db");
    col_db.migrate().expect("failed to migrate col db");
    let col_strava = gpx_to_graph::col::strava::StravaConfig::from_env();
    let col_state: gpx_to_graph::col::SharedState = std::sync::Arc::new(
        gpx_to_graph::col::AppState {
            db: col_db,
            strava: col_strava,
        },
    );

    // --- Trip service ---
    let trip_db_path = std::env::var("TRIP_DB_PATH").unwrap_or_else(|_| "data/trip.db".into());
    if let Some(parent) = std::path::Path::new(&trip_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let trip_db = gpx_to_graph::trip::db::Db::open(&trip_db_path)
        .expect("failed to open trip db");
    trip_db.migrate().expect("failed to migrate trip db");
    let trip_state: gpx_to_graph::trip::SharedState = std::sync::Arc::new(
        gpx_to_graph::trip::AppState {
            db: trip_db,
        },
    );

    // --- Merge sessions ---
    let merge_sessions: MergeSessions = Arc::new(Mutex::new(HashMap::new()));

    // --- Build unified app ---
    let app = Router::new()
        // Original gpx_to_graph routes
        .route("/", get(form_page))
        .route("/generate", post(generate_handler))
        .route("/merge", post(merge_handler))
        .route("/merge/upload", post(merge_upload_handler))
        .route("/merge/run", post(merge_run_handler))
        .route("/share/{id}", get(share_page))
        .route(
            "/share/{id}/{file}",
            get(share_file).options(share_file_options),
        )
        .route("/static/recents.css", get(static_recents_css))
        .route("/static/recents.js", get(static_recents_js))
        .route("/static/themes.css", get(static_themes_css))
        .route("/static/theme-switcher.js", get(static_theme_js))
        .route("/toolkit/simplify", post(simplify_handler))
        .layer(DefaultBodyLimit::max(500 * 1024 * 1024))
        .with_state(merge_sessions)
        // Nested service routers
        .nest("/meteo", gpx_to_graph::meteo::router(meteo_cache))
        .nest("/ravito", gpx_to_graph::ravito::router(ravito_cache))
        .nest("/trace", gpx_to_graph::trace::router(trace_state))
        .nest("/stats", gpx_to_graph::strava_stats::router())
        .nest("/col", gpx_to_graph::col::router(col_state))
        .nest("/toolkit", gpx_to_graph::toolkit::router())
        .nest("/trip", gpx_to_graph::trip::router(trip_state));

    // Purge share directories older than SHARE_TTL_SECS every 10 min.
    tokio::spawn(async {
        loop {
            cleanup_shares().await;
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Server running at http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");
    axum::serve(listener, app)
        .await
        .expect("server error");
}
