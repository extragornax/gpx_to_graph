use std::io::Cursor;
use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use gpx_to_graph::{generate, GeneratedOutput, GraphOptions};
use serde_json::{json, Value};

const SHARE_TTL_SECS: u64 = 48 * 3600;

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
        for i in 0..12 {
            buf[i] = ((n >> (i * 8)) & 0xff) as u8;
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
<title>GPX to Graph</title>
<style>
  *, *::before, *::after { box-sizing: border-box; }
  body {
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: #f0f2f5;
    color: #1a1a1a;
    margin: 0;
    padding: 2rem 1rem;
  }
  .container {
    max-width: 900px;
    margin: 0 auto;
  }
  h1 {
    font-size: 1.75rem;
    font-weight: 700;
    margin: 0 0 0.25rem;
  }
  .subtitle {
    color: #666;
    margin: 0 0 2rem;
    font-size: 0.95rem;
  }
  .card {
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
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
    border: 2px dashed #ccc;
    border-radius: 8px;
    background: #fafafa;
    cursor: pointer;
    font-size: 0.9rem;
  }
  .file-section input[type="file"]:hover {
    border-color: #999;
  }
  .section-title {
    font-weight: 600;
    font-size: 0.95rem;
    margin: 1.5rem 0 1rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid #eee;
    color: #444;
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
    color: #555;
    margin-bottom: 0.35rem;
  }
  .field input[type="number"],
  .field input[type="text"] {
    padding: 0.6rem 0.75rem;
    border: 1px solid #d0d0d0;
    border-radius: 6px;
    font-size: 0.9rem;
    background: #fafafa;
    transition: border-color 0.15s;
  }
  .field input:focus {
    outline: none;
    border-color: #4a90d9;
    box-shadow: 0 0 0 3px rgba(74,144,217,0.12);
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
  }
  .checkbox-field label {
    font-size: 0.9rem;
    cursor: pointer;
    color: #333;
  }
  .hint {
    font-size: 0.75rem;
    color: #888;
    margin-top: 0.25rem;
  }
  .submit-section {
    margin-top: 2rem;
    text-align: right;
  }
  button[type="submit"] {
    background: #2563eb;
    color: #fff;
    border: none;
    padding: 0.75rem 2rem;
    font-size: 1rem;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s;
  }
  button[type="submit"]:hover {
    background: #1d4ed8;
  }
  .tabs {
    display: flex;
    gap: 0.5rem;
    border-bottom: 2px solid #e5e7eb;
    margin-bottom: 1.5rem;
  }
  .tab {
    background: transparent;
    border: none;
    padding: 0.75rem 1.5rem;
    font-size: 0.95rem;
    font-weight: 600;
    color: #666;
    cursor: pointer;
    border-bottom: 3px solid transparent;
    margin-bottom: -2px;
    transition: color 0.15s, border-color 0.15s;
  }
  .tab:hover { color: #333; }
  .tab.active {
    color: #2563eb;
    border-bottom-color: #2563eb;
  }
  .panel { display: none; }
  .panel.active { display: block; }
  .status-line {
    margin-top: 1rem;
    font-weight: 600;
    font-size: 0.95rem;
  }
  .status-line.info { color: #2563eb; }
  .status-line.ok { color: #16a34a; }
  .status-line.err { color: #dc2626; }
  .merge-results { margin-top: 2rem; display: none; }
  .merge-results.visible { display: block; }
  .merge-results h2 {
    font-size: 1.15rem;
    font-weight: 700;
    margin: 0 0 1rem;
    color: #111;
  }
  .stat-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 0.75rem;
  }
  .stat-box {
    background: #f8fafc;
    border: 1px solid #e5e7eb;
    border-radius: 10px;
    padding: 0.85rem 1rem;
  }
  .stat-box .stat-name {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #64748b;
    font-weight: 600;
  }
  .stat-box .stat-val {
    font-size: 1.35rem;
    font-weight: 700;
    color: #1d4ed8;
    margin-top: 0.2rem;
    line-height: 1.1;
  }
  .stat-box .stat-sub {
    font-size: 0.78rem;
    color: #6b7280;
    margin-top: 0.25rem;
  }
  .chart-section {
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid #eee;
  }
  .chart-section label {
    font-size: 0.85rem;
    font-weight: 600;
    color: #333;
    margin-right: 0.5rem;
  }
  .chart-section select {
    padding: 0.45rem 0.6rem;
    border: 1px solid #d0d0d0;
    border-radius: 6px;
    font-size: 0.9rem;
    background: #fff;
  }
  .chart {
    display: block;
    width: 100%;
    height: 240px;
    margin-top: 0.75rem;
    background: #fff;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
  }
  .chart-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 1rem 1.75rem;
  }
  .chart-controls .cc-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .chart-controls label {
    font-size: 0.85rem;
    font-weight: 600;
    color: #333;
    margin: 0;
  }
  .chart-controls input[type="range"] {
    width: 160px;
    accent-color: #2563eb;
  }
  .chart-controls .cb-label {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    user-select: none;
  }
  .chart-controls .cb-label input[type="checkbox"] {
    accent-color: #2563eb;
    margin: 0;
  }
  #smoothVal {
    display: inline-block;
    min-width: 1.5rem;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: #2563eb;
    font-weight: 700;
  }
  .file-section.dragging input[type="file"] {
    border-color: #2563eb;
    background: #eff6ff;
  }
  .per-file-section {
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid #eee;
  }
  .per-file-section h3 {
    font-size: 1rem;
    font-weight: 700;
    margin: 0 0 0.75rem;
    color: #111;
  }
  .per-file-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .per-file-card {
    background: #f8fafc;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    padding: 0.75rem 1rem;
  }
  .pf-name {
    font-weight: 600;
    font-size: 0.9rem;
    color: #111;
    margin-bottom: 0.4rem;
    word-break: break-all;
  }
  .pf-metrics {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.6rem;
    font-size: 0.8rem;
    color: #374151;
  }
  .pf-metrics span {
    background: #fff;
    border: 1px solid #e5e7eb;
    border-radius: 5px;
    padding: 0.15rem 0.5rem;
  }
  .pf-metrics b {
    color: #1d4ed8;
    font-weight: 700;
  }
</style>
</head>
<body>
<div class="container">
  <h1>GPX Tools</h1>
  <p class="subtitle">Generate elevation graphs or merge multiple GPX files.</p>

  <div class="tabs">
    <button type="button" class="tab active" data-target="graph">Graph</button>
    <button type="button" class="tab" data-target="merge">Merge</button>
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

  <div id="panel-merge" class="panel">
    <div class="card">
      <p class="subtitle" style="margin-bottom: 1.5rem;">Select 2 to 5 GPX files. They are sorted by timestamp and merged into a single track.</p>
      <form id="mergeForm" enctype="multipart/form-data">
        <div class="file-section">
          <label for="merge_files">GPX Files</label>
          <input type="file" id="merge_files" name="files" accept=".gpx" multiple required>
          <span class="hint">Hold Ctrl/Cmd to select multiple files (2–5).</span>
        </div>
        <div class="field">
          <label for="creator">Device / Creator Name</label>
          <input type="text" id="creator" name="creator" placeholder="e.g. Garmin Edge 1040">
          <span class="hint">Leave empty to keep the first file's creator value.</span>
        </div>
        <div class="submit-section">
          <button type="submit">Merge &amp; Download</button>
        </div>
        <div id="mergeStatus" class="status-line"></div>
      </form>
      <div id="mergeResults" class="merge-results">
        <h2>Track Summary</h2>
        <div id="mergeStatGrid" class="stat-grid"></div>
        <div id="chartSection" class="chart-section">
          <div class="chart-controls">
            <div class="cc-item">
              <label for="metricSelect">Metric</label>
              <select id="metricSelect"></select>
            </div>
            <div class="cc-item">
              <label for="smoothRange">Smoothing <span id="smoothVal">0</span></label>
              <input type="range" id="smoothRange" min="0" max="30" step="1" value="0">
            </div>
            <div class="cc-item">
              <label for="removeZeros" class="cb-label">
                <input type="checkbox" id="removeZeros">
                Ignore zero values
              </label>
            </div>
          </div>
          <svg id="metricChart" class="chart" xmlns="http://www.w3.org/2000/svg"
               viewBox="0 0 800 240" preserveAspectRatio="none"></svg>
        </div>
        <div id="perFileSection" class="per-file-section" style="display:none;">
          <h3>Per-file breakdown</h3>
          <div id="perFileList" class="per-file-list"></div>
        </div>
      </div>
    </div>
  </div>
</div>
<script>
  document.querySelectorAll('.tab').forEach(function (tab) {
    tab.addEventListener('click', function () {
      document.querySelectorAll('.tab').forEach(function (t) { t.classList.remove('active'); });
      document.querySelectorAll('.panel').forEach(function (p) { p.classList.remove('active'); });
      tab.classList.add('active');
      document.getElementById('panel-' + tab.dataset.target).classList.add('active');
    });
  });

  var mergeForm = document.getElementById('mergeForm');
  var mergeStatus = document.getElementById('mergeStatus');
  var mergeResults = document.getElementById('mergeResults');
  var mergeStatGrid = document.getElementById('mergeStatGrid');
  var chartSection = document.getElementById('chartSection');
  var metricSelect = document.getElementById('metricSelect');
  var metricChart = document.getElementById('metricChart');
  var lastSeries = null;
  var lastLabels = {};

  function fmtNum(v, digits) {
    if (v === null || v === undefined || !isFinite(v)) return '—';
    return Number(v).toFixed(digits == null ? 1 : digits);
  }

  function fmtDuration(s) {
    if (!isFinite(s) || s <= 0) return '—';
    var total = Math.round(s);
    var h = Math.floor(total / 3600);
    var m = Math.floor((total % 3600) / 60);
    var sec = total % 60;
    var pad = function (n) { return String(n).padStart(2, '0'); };
    if (h > 0) return h + 'h ' + pad(m) + 'm';
    return m + 'm ' + pad(sec) + 's';
  }

  function fmtPace(kmh) {
    if (!isFinite(kmh) || kmh <= 0) return '—';
    var sPerKm = 3600 / kmh;
    var m = Math.floor(sPerKm / 60);
    var s = Math.round(sPerKm % 60);
    if (s === 60) { m += 1; s = 0; }
    return m + ':' + String(s).padStart(2, '0') + ' /km';
  }

  function statBox(name, val, sub) {
    var box = document.createElement('div');
    box.className = 'stat-box';
    var inner = '<div class="stat-name">' + name + '</div><div class="stat-val">' + val + '</div>';
    if (sub) inner += '<div class="stat-sub">' + sub + '</div>';
    box.innerHTML = inner;
    mergeStatGrid.appendChild(box);
  }

  function renderStats(stats) {
    mergeStatGrid.innerHTML = '';
    statBox('Total distance', fmtNum(stats.total_km, 2) + ' km');
    statBox('Track points', stats.point_count.toLocaleString());
    if (stats.total_time_s > 0) {
      statBox('Total time', fmtDuration(stats.total_time_s));
      statBox('Moving time', fmtDuration(stats.moving_time_s),
        'idle ' + fmtDuration(stats.idle_time_s));
      statBox('Avg speed', fmtNum(stats.avg_moving_speed_kmh, 1) + ' km/h',
        'overall ' + fmtNum(stats.avg_speed_kmh, 1) + ' km/h');
      statBox('Avg pace', fmtPace(stats.avg_moving_speed_kmh));
    }
    if (stats.elevation) {
      statBox(
        'Elevation gain',
        '+' + fmtNum(stats.elevation.gain, 0) + ' m',
        'loss ' + fmtNum(stats.elevation.loss, 0) + ' m'
      );
      statBox(
        'Elevation range',
        fmtNum(stats.elevation.min, 0) + '–' + fmtNum(stats.elevation.max, 0) + ' m',
        'avg ' + fmtNum(stats.elevation.avg, 0) + ' m'
      );
    }
    if (stats.hr) statBox('Avg heart rate', fmtNum(stats.hr.avg, 0) + ' bpm',
        'min ' + fmtNum(stats.hr.min, 0) + ' · max ' + fmtNum(stats.hr.max, 0));
    if (stats.cadence) statBox('Avg cadence', fmtNum(stats.cadence.avg, 0) + ' rpm',
        'max ' + fmtNum(stats.cadence.max, 0));
    if (stats.power) statBox('Avg power', fmtNum(stats.power.avg, 0) + ' W',
        'max ' + fmtNum(stats.power.max, 0));
    if (stats.temperature) statBox('Avg temperature', fmtNum(stats.temperature.avg, 1) + ' °C',
        fmtNum(stats.temperature.min, 1) + '–' + fmtNum(stats.temperature.max, 1) + ' °C');

    lastSeries = stats.series;
    lastLabels = {
      elevation: 'Elevation (m)',
      hr: 'Heart Rate (bpm)',
      cadence: 'Cadence (rpm)',
      power: 'Power (W)',
      temperature: 'Temperature (°C)'
    };

    var metrics = ['elevation', 'hr', 'cadence', 'power', 'temperature']
      .filter(function (k) { return stats.series[k] && stats.series[k].km && stats.series[k].km.length > 1; });

    metricSelect.innerHTML = '';
    if (metrics.length === 0) {
      chartSection.style.display = 'none';
      return;
    }
    chartSection.style.display = '';
    metrics.forEach(function (k) {
      var opt = document.createElement('option');
      opt.value = k;
      opt.textContent = lastLabels[k];
      metricSelect.appendChild(opt);
    });
    metricSelect.value = metrics[0];
    drawChart(metrics[0]);
  }

  function movingAverage(values, radius) {
    var n = values.length;
    if (radius <= 0 || n <= 1) return values.slice();
    var out = new Array(n);
    var sum = 0, count = 0;
    for (var i = 0; i < Math.min(radius, n); i++) { sum += values[i]; count++; }
    for (var i = 0; i < n; i++) {
      if (i + radius < n) { sum += values[i + radius]; count++; }
      if (i - radius - 1 >= 0) { sum -= values[i - radius - 1]; count--; }
      out[i] = sum / count;
    }
    return out;
  }

  function drawChart(key) {
    var data = lastSeries && lastSeries[key];
    if (!data || !data.km || data.km.length < 2) { metricChart.innerHTML = ''; return; }
    var W = 800, H = 240;
    var padL = 52, padR = 18, padT = 22, padB = 32;
    var xs = data.km, ys = data.value;
    if (removeZeros && removeZeros.checked) {
      var fxs = [], fys = [];
      for (var k = 0; k < ys.length; k++) {
        if (ys[k] !== 0) { fxs.push(xs[k]); fys.push(ys[k]); }
      }
      xs = fxs; ys = fys;
    }
    if (xs.length < 2) { metricChart.innerHTML = ''; return; }
    var radius = smoothRange ? (parseInt(smoothRange.value, 10) || 0) : 0;
    if (radius > 0) ys = movingAverage(ys, radius);
    var minX = xs[0], maxX = xs[xs.length - 1];
    var minY = Infinity, maxY = -Infinity;
    for (var i = 0; i < ys.length; i++) {
      if (ys[i] < minY) minY = ys[i];
      if (ys[i] > maxY) maxY = ys[i];
    }
    if (minY === maxY) { minY -= 1; maxY += 1; }
    var mx = function (x) { return padL + (x - minX) / (maxX - minX || 1) * (W - padL - padR); };
    var my = function (y) { return H - padB - (y - minY) / (maxY - minY) * (H - padT - padB); };
    var pts = '';
    for (var j = 0; j < xs.length; j++) {
      pts += (j ? ' ' : '') + mx(xs[j]).toFixed(1) + ',' + my(ys[j]).toFixed(1);
    }
    var midY = (minY + maxY) / 2;
    var gridYs = [maxY, midY, minY];
    var gridLines = gridYs.map(function (gv) {
      var y = my(gv).toFixed(1);
      return '<line x1="' + padL + '" y1="' + y + '" x2="' + (W - padR) + '" y2="' + y +
             '" stroke="#eef2f7" stroke-width="1"/>' +
             '<text x="' + (padL - 6) + '" y="' + (parseFloat(y) + 3) + '" text-anchor="end" font-size="10" fill="#6b7280">' +
             fmtNum(gv, Math.abs(maxY - minY) < 10 ? 1 : 0) + '</text>';
    }).join('');
    var midX = (minX + maxX) / 2;
    var xTicks = [minX, midX, maxX].map(function (xv) {
      var x = mx(xv).toFixed(1);
      return '<text x="' + x + '" y="' + (H - 10) + '" text-anchor="middle" font-size="10" fill="#6b7280">' +
             fmtNum(xv, 1) + ' km</text>';
    }).join('');
    metricChart.innerHTML =
      '<rect x="' + padL + '" y="' + padT + '" width="' + (W - padL - padR) + '" height="' + (H - padT - padB) +
      '" fill="#fafbfc" stroke="#e5e7eb"/>' +
      gridLines +
      '<polyline points="' + pts + '" fill="none" stroke="#2563eb" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/>' +
      xTicks +
      '<text x="' + padL + '" y="' + (padT - 6) + '" font-size="11" font-weight="600" fill="#111">' +
      (lastLabels[key] || key) + '</text>';
  }

  var smoothRange = document.getElementById('smoothRange');
  var smoothVal = document.getElementById('smoothVal');
  var removeZeros = document.getElementById('removeZeros');
  if (smoothRange && smoothVal) {
    smoothRange.addEventListener('input', function () {
      smoothVal.textContent = smoothRange.value;
      drawChart(metricSelect.value);
    });
  }
  if (removeZeros) {
    removeZeros.addEventListener('change', function () {
      drawChart(metricSelect.value);
    });
  }

  metricSelect.addEventListener('change', function () { drawChart(metricSelect.value); });

  function renderPerFile(entries) {
    var section = document.getElementById('perFileSection');
    var list = document.getElementById('perFileList');
    list.innerHTML = '';
    if (!entries || !entries.length) { section.style.display = 'none'; return; }
    section.style.display = '';
    entries.forEach(function (entry) {
      var s = entry.stats || {};
      var card = document.createElement('div');
      card.className = 'per-file-card';
      var parts = [];
      parts.push('<span>Dist: <b>' + fmtNum(s.total_km, 2) + ' km</b></span>');
      parts.push('<span>Points: <b>' + (s.point_count || 0).toLocaleString() + '</b></span>');
      if (s.moving_time_s > 0) {
        parts.push('<span>Moving: <b>' + fmtDuration(s.moving_time_s) + '</b></span>');
        if (s.avg_moving_speed_kmh > 0) {
          parts.push('<span>Avg: <b>' + fmtNum(s.avg_moving_speed_kmh, 1) + ' km/h</b></span>');
          parts.push('<span>Pace: <b>' + fmtPace(s.avg_moving_speed_kmh) + '</b></span>');
        }
      }
      if (s.elevation && s.elevation.gain !== undefined) {
        parts.push('<span>Gain: <b>+' + fmtNum(s.elevation.gain, 0) + ' m</b></span>');
      }
      if (s.hr && s.hr.avg !== undefined) parts.push('<span>HR: <b>' + fmtNum(s.hr.avg, 0) + ' bpm</b></span>');
      if (s.cadence && s.cadence.avg !== undefined) parts.push('<span>Cad: <b>' + fmtNum(s.cadence.avg, 0) + '</b></span>');
      if (s.power && s.power.avg !== undefined) parts.push('<span>Pow: <b>' + fmtNum(s.power.avg, 0) + ' W</b></span>');
      if (s.temperature && s.temperature.avg !== undefined) parts.push('<span>Temp: <b>' + fmtNum(s.temperature.avg, 1) + ' °C</b></span>');
      card.innerHTML =
        '<div class="pf-name">' + (entry.name || '(unnamed)') + '</div>' +
        '<div class="pf-metrics">' + parts.join('') + '</div>';
      list.appendChild(card);
    });
  }

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
        if (ok) {
          if (!input.multiple && accepted >= 1) break;
          dt.items.add(f);
          accepted++;
        }
      }
      if (accepted === 0) return;
      input.files = dt.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });
  }
  ['gpx_file', 'merge_files'].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) enableDropZone(el);
  });

  function triggerDownload(gpxString) {
    var blob = new Blob([gpxString], { type: 'application/gpx+xml' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'merged.gpx';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  mergeForm.addEventListener('submit', async function (e) {
    e.preventDefault();
    var files = document.getElementById('merge_files').files;
    if (files.length < 2 || files.length > 5) {
      mergeStatus.textContent = 'Please select between 2 and 5 GPX files.';
      mergeStatus.className = 'status-line err';
      return;
    }
    mergeStatus.textContent = 'Merging…';
    mergeStatus.className = 'status-line info';
    mergeResults.classList.remove('visible');
    try {
      var res = await fetch('/merge', { method: 'POST', body: new FormData(mergeForm) });
      if (!res.ok) {
        var errText = await res.text();
        throw new Error(errText || ('Request failed: ' + res.status));
      }
      var payload = await res.json();
      if (!payload.gpx) throw new Error('Empty response');
      triggerDownload(payload.gpx);
      renderStats(payload.stats || {});
      renderPerFile(payload.per_file || []);
      mergeResults.classList.add('visible');
      mergeStatus.textContent = 'Merged successfully — merged.gpx downloaded.';
      mergeStatus.className = 'status-line ok';
    } catch (err) {
      mergeStatus.textContent = 'Error: ' + err.message;
      mergeStatus.className = 'status-line err';
    }
  });
</script>
</body>
</html>"##;

async fn form_page() -> Html<&'static str> {
    Html(FORM_HTML)
}

fn error_page(message: &str) -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Error - GPX to Graph</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: #f0f2f5;
    color: #1a1a1a;
    margin: 0;
    padding: 2rem 1rem;
  }}
  .container {{
    max-width: 900px;
    margin: 0 auto;
  }}
  h1 {{
    font-size: 1.75rem;
    font-weight: 700;
    color: #dc2626;
    margin: 0 0 1rem;
  }}
  .card {{
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
    padding: 2rem;
  }}
  .error-message {{
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 8px;
    padding: 1rem 1.25rem;
    color: #991b1b;
    font-size: 0.95rem;
    margin-bottom: 1.5rem;
    white-space: pre-wrap;
    word-break: break-word;
  }}
  a {{
    color: #2563eb;
    text-decoration: none;
    font-weight: 600;
  }}
  a:hover {{
    text-decoration: underline;
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

async fn save_share(gpx_bytes: &[u8], output: &GeneratedOutput) -> Result<String, String> {
    let id = random_id();
    let dir = share_dir().join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create_dir_all: {e}"))?;

    tokio::fs::write(dir.join("source.gpx"), gpx_bytes)
        .await
        .map_err(|e| format!("write source.gpx: {e}"))?;

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
        if let Ok(meta_str) = tokio::fs::read_to_string(path.join("meta.json")).await {
            if let Ok(v) = serde_json::from_str::<Value>(&meta_str) {
                if let Some(created_at) = v.get("created_at").and_then(|v| v.as_u64()) {
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
            }
        }
        // Fallback: mtime.
        if !old {
            if let Ok(md) = tokio::fs::metadata(&path).await {
                if let Ok(modified) = md.modified() {
                    if now
                        .duration_since(modified)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                        > SHARE_TTL_SECS
                    {
                        old = true;
                    }
                }
            }
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
    let hours_left = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if expires_at > now {
            (expires_at - now) / 3600
        } else {
            0
        }
    };
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

    let og_title = format!("GPX route — {total_km:.1} km");
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
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: #f0f2f5;
    color: #1a1a1a;
    margin: 0;
    padding: 2rem 1rem;
  }}
  .container {{ max-width: 900px; margin: 0 auto; }}
  h1 {{ font-size: 1.75rem; font-weight: 700; margin: 0 0 0.25rem; }}
  .back-link {{
    display: inline-block;
    margin-bottom: 1.5rem;
    color: #2563eb;
    text-decoration: none;
    font-weight: 600;
    font-size: 0.95rem;
  }}
  .back-link:hover {{ text-decoration: underline; }}
  .card {{
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
    padding: 1.5rem 2rem;
    margin-bottom: 1.5rem;
  }}
  .summary {{ display: flex; gap: 2rem; flex-wrap: wrap; }}
  .stat {{ text-align: center; }}
  .stat-value {{ font-size: 1.5rem; font-weight: 700; color: #2563eb; }}
  .stat-label {{
    font-size: 0.8rem; color: #666;
    text-transform: uppercase; letter-spacing: 0.05em; margin-top: 0.2rem;
  }}
  .share-banner {{
    display: flex; flex-direction: column; gap: 0.6rem;
    border: 1px solid #dbeafe; background: #eff6ff;
  }}
  .share-banner .share-title {{ font-weight: 700; font-size: 0.95rem; color: #1e3a8a; }}
  .share-banner .share-row {{ display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap; }}
  .share-banner input[type="text"] {{
    flex: 1 1 260px;
    min-width: 0;
    padding: 0.55rem 0.75rem;
    border: 1px solid #bfdbfe;
    border-radius: 6px;
    font: inherit;
    background: #fff;
    color: #111;
  }}
  .share-banner button {{
    padding: 0.55rem 0.9rem;
    background: #2563eb;
    color: #fff;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 600;
    font-size: 0.9rem;
  }}
  .share-banner button:hover {{ background: #1d4ed8; }}
  .share-banner .ttl-note {{ font-size: 0.82rem; color: #4b5563; margin: 0; }}
  .share-banner a {{ color: #2563eb; font-weight: 600; text-decoration: none; font-size: 0.9rem; }}
  .share-banner a:hover {{ text-decoration: underline; }}
  .share-banner a.btn-link {{
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.55rem 0.9rem;
    background: #fff;
    color: #2563eb;
    border: 1px solid #2563eb;
    border-radius: 6px;
    font-weight: 600;
    font-size: 0.9rem;
    text-decoration: none;
    white-space: nowrap;
  }}
  .share-banner a.btn-link:hover {{ background: #eff6ff; text-decoration: none; }}
  .image-card {{
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }}
  .image-label {{ font-weight: 600; font-size: 1rem; margin-bottom: 1rem; color: #333; }}
  .image-card img {{ max-width: 100%; height: auto; border-radius: 8px; border: 1px solid #e5e7eb; }}
  .download-link {{
    display: inline-block;
    margin-top: 0.75rem;
    color: #2563eb;
    text-decoration: none;
    font-weight: 500;
    font-size: 0.9rem;
  }}
  .download-link:hover {{ text-decoration: underline; }}
</style>
</head>
<body>
<div class="container">
  <h1>Generated Profile</h1>
  <a class="back-link" href="/">&larr; Generate another</a>

  <div class="card share-banner">
    <div class="share-title">Share this result</div>
    <div class="share-row">
      <input type="text" id="shareUrl" readonly>
      <button id="copyBtn" type="button">Copy link</button>
      <a class="btn-link" href="{gpx_studio_url}" target="_blank" rel="noopener">Open in gpx.studio &rarr;</a>
    </div>
    <p class="ttl-note">Link expires in about {hours_left} hour(s). Results and the source GPX are kept for 48 h after creation.</p>
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
  var shareInput = document.getElementById('shareUrl');
  shareInput.value = window.location.href;
  document.getElementById('copyBtn').addEventListener('click', async function () {{
    var btn = this;
    try {{
      await navigator.clipboard.writeText(shareInput.value);
    }} catch (e) {{
      shareInput.select();
      document.execCommand('copy');
    }}
    var prev = btn.textContent;
    btn.textContent = 'Copied!';
    setTimeout(function () {{ btn.textContent = prev; }}, 1500);
  }});
</script>
</body>
</html>"#,
        id = id,
        total_km = total_km,
        num_checkpoints = num_checkpoints,
        num_climbs = num_climbs,
        hours_left = hours_left,
        images_html = images_html,
        og_meta = og_meta,
        gpx_studio_url = gpx_studio_url,
    )
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

    Ok(splice_trksegs_before_first_close_trk(&base, &extras))
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

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    let app = Router::new()
        .route("/", get(form_page))
        .route("/generate", post(generate_handler))
        .route("/merge", post(merge_handler))
        .route("/share/{id}", get(share_page))
        .route(
            "/share/{id}/{file}",
            get(share_file).options(share_file_options),
        )
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024));

    // Purge share directories older than SHARE_TTL_SECS every 10 min.
    tokio::spawn(async {
        loop {
            cleanup_shares().await;
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    println!(
        "Server running at http://localhost:{port} (shares in {})",
        share_dir().display()
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");
    axum::serve(listener, app)
        .await
        .expect("server error");
}
