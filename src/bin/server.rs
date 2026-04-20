use std::io::Cursor;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use gpx::{read as gpx_read, write as gpx_write, Gpx};
use gpx_to_graph::{generate, GeneratedOutput, GraphOptions};

const FORM_HTML: &str = r#"<!DOCTYPE html>
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
    try {
      var res = await fetch('/merge', { method: 'POST', body: new FormData(mergeForm) });
      if (!res.ok) {
        var errText = await res.text();
        throw new Error(errText || ('Request failed: ' + res.status));
      }
      var blob = await res.blob();
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = 'merged.gpx';
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      mergeStatus.textContent = 'Merged successfully — merged.gpx downloaded.';
      mergeStatus.className = 'status-line ok';
    } catch (err) {
      mergeStatus.textContent = 'Error: ' + err.message;
      mergeStatus.className = 'status-line err';
    }
  });
</script>
</body>
</html>"#;

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

async fn generate_handler(mut multipart: Multipart) -> Html<String> {
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
        None => return error_page("No GPX file was uploaded."),
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

    let result = tokio::task::spawn_blocking(move || {
        let reader = Cursor::new(gpx_bytes);
        generate(reader, &opts)
    })
    .await;

    let output: GeneratedOutput = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return error_page(&format!("{e:#}")),
        Err(e) => return error_page(&format!("Task failed: {e}")),
    };

    Html(build_results_page(&output))
}

fn build_results_page(output: &GeneratedOutput) -> String {
    let mut images_html = String::new();

    for (i, (label, png_bytes)) in output.graph_images.iter().enumerate() {
        let b64 = STANDARD.encode(png_bytes);
        let safe_label = html_escape(label);
        let download_name = if output.graph_images.len() == 1 {
            "profile.png".to_string()
        } else {
            format!("profile_{}.png", i + 1)
        };

        images_html.push_str(&format!(
            r#"
      <div class="image-card">
        <div class="image-label">{safe_label}</div>
        <img src="data:image/png;base64,{b64}" alt="{safe_label}">
        <a class="download-link" href="data:image/png;base64,{b64}" download="{download_name}">Download {safe_label}</a>
      </div>"#
        ));
    }

    if let Some(ref stats_bytes) = output.climb_stats {
        let b64 = STANDARD.encode(stats_bytes);
        images_html.push_str(&format!(
            r#"
      <div class="image-card">
        <div class="image-label">Climb Statistics</div>
        <img src="data:image/png;base64,{b64}" alt="Climb Statistics">
        <a class="download-link" href="data:image/png;base64,{b64}" download="climb_stats.png">Download Climb Statistics</a>
      </div>"#
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Results - GPX to Graph</title>
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
    margin: 0 0 0.25rem;
  }}
  .back-link {{
    display: inline-block;
    margin-bottom: 1.5rem;
    color: #2563eb;
    text-decoration: none;
    font-weight: 600;
    font-size: 0.95rem;
  }}
  .back-link:hover {{
    text-decoration: underline;
  }}
  .card {{
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
    padding: 2rem;
    margin-bottom: 1.5rem;
  }}
  .summary {{
    display: flex;
    gap: 2rem;
    flex-wrap: wrap;
  }}
  .stat {{
    text-align: center;
  }}
  .stat-value {{
    font-size: 1.5rem;
    font-weight: 700;
    color: #2563eb;
  }}
  .stat-label {{
    font-size: 0.8rem;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-top: 0.2rem;
  }}
  .image-card {{
    background: #fff;
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.08), 0 4px 12px rgba(0,0,0,0.04);
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }}
  .image-label {{
    font-weight: 600;
    font-size: 1rem;
    margin-bottom: 1rem;
    color: #333;
  }}
  .image-card img {{
    max-width: 100%;
    height: auto;
    border-radius: 8px;
    border: 1px solid #e5e7eb;
  }}
  .download-link {{
    display: inline-block;
    margin-top: 0.75rem;
    color: #2563eb;
    text-decoration: none;
    font-weight: 500;
    font-size: 0.9rem;
  }}
  .download-link:hover {{
    text-decoration: underline;
  }}
</style>
</head>
<body>
<div class="container">
  <h1>Generated Profile</h1>
  <a class="back-link" href="/">&larr; Generate another</a>

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
</body>
</html>"#,
        total_km = output.total_km,
        num_checkpoints = output.num_checkpoints,
        num_climbs = output.num_climbs,
        images_html = images_html,
    )
}

fn sort_by_first_time(data: &[Gpx]) -> Vec<Gpx> {
    let mut cloned = data.to_owned();
    cloned.sort_by_key(|item| {
        item.tracks
            .first()
            .and_then(|t| t.segments.first())
            .and_then(|s| s.points.first())
            .and_then(|p| p.time)
    });
    cloned
}

fn merge_traces(data: &[Gpx], creator: Option<String>) -> Gpx {
    if data.is_empty() {
        return Gpx::default();
    }
    if data.len() == 1 {
        let mut single = data[0].clone();
        if let Some(cc) = creator {
            single.creator = Some(cc);
        }
        return single;
    }

    let sorted = sort_by_first_time(data);
    let (base, remaining) = sorted.split_at(1);
    let mut base = base[0].clone();
    for item in remaining {
        for lt in &item.tracks {
            if base.tracks.is_empty() {
                base.tracks.push(lt.clone());
            } else {
                for ls in &lt.segments {
                    base.tracks[0].segments.push(ls.clone());
                }
            }
        }
    }

    if let Some(cc) = creator {
        base.creator = Some(cc);
    }
    base
}

async fn merge_handler(mut multipart: Multipart) -> Response {
    let mut files: Vec<Vec<u8>> = Vec::new();
    let mut creator: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "files" => {
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    files.push(data.to_vec());
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
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let mut parsed: Vec<Gpx> = Vec::with_capacity(files.len());
        for (i, bytes) in files.iter().enumerate() {
            let g = gpx_read(std::io::Cursor::new(bytes))
                .map_err(|e| format!("Invalid GPX in file {}: {e}", i + 1))?;
            parsed.push(g);
        }
        let merged = merge_traces(&parsed, creator_for_task);
        let mut out: Vec<u8> = Vec::new();
        gpx_write(&merged, &mut out)
            .map_err(|e| format!("Failed to serialize merged GPX: {e}"))?;
        Ok(out)
    })
    .await;

    match result {
        Ok(Ok(bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/gpx+xml")
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"merged.gpx\"",
            )
            .body(Body::from(bytes))
            .expect("valid response"),
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
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024));

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    println!("Server running at http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");
    axum::serve(listener, app)
        .await
        .expect("server error");
}
