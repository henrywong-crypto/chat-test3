use maud::{html, Markup, PreEscaped, DOCTYPE};

// Shared CSS design tokens and component styles
fn base_styles() -> &'static str {
    "
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    :root {
        --bg:         #0d1117;
        --surface:    #161b22;
        --surface2:   #1c2128;
        --border:     #30363d;
        --text:       #e6edf3;
        --text-muted: #8b949e;
        --accent:     #58a6ff;
        --success:    #3fb950;
        --danger:     #f85149;
        --radius:     6px;
        --font:       ui-monospace, 'SFMono-Regular', 'Menlo', 'Consolas', monospace;
    }
    body { background: var(--bg); color: var(--text); font-family: var(--font); font-size: 13px; }
    a { color: var(--accent); text-decoration: none; }
    a:hover { text-decoration: underline; }

    .btn {
        display: inline-flex; align-items: center; gap: 6px;
        padding: 5px 12px; border-radius: var(--radius);
        border: 1px solid var(--border); background: var(--surface2);
        color: var(--text); font-family: var(--font); font-size: 12px;
        cursor: pointer; white-space: nowrap;
    }
    .btn:hover { border-color: var(--text-muted); }
    .btn-primary { background: #1f6feb; border-color: #388bfd; color: #fff; }
    .btn-primary:hover { background: #388bfd; }
    .btn-danger  { background: transparent; border-color: var(--danger); color: var(--danger); }
    .btn-danger:hover  { background: #3d0f0f; }
    .btn-ghost   { background: transparent; border-color: transparent; color: var(--text-muted); }
    .btn-ghost:hover   { color: var(--text); background: var(--surface2); }

    input[type=text] {
        background: var(--surface); border: 1px solid var(--border);
        border-radius: var(--radius); color: var(--text);
        font-family: var(--font); font-size: 12px; padding: 5px 8px;
    }
    input[type=text]:focus { outline: none; border-color: var(--accent); }

    .badge {
        display: inline-flex; align-items: center;
        padding: 1px 7px; border-radius: 10px; font-size: 11px;
        border: 1px solid transparent;
    }
    .badge-green  { background: #0d2918; border-color: #26a641; color: var(--success); }
    .badge-gray   { background: var(--surface2); border-color: var(--border); color: var(--text-muted); }
    "
}

pub(crate) fn render_login_page() -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "WebCode" }
                style { (PreEscaped(base_styles())) (PreEscaped("
                    body { display: flex; align-items: center; justify-content: center; min-height: 100vh; }
                    .login-card {
                        width: 320px; background: var(--surface); border: 1px solid var(--border);
                        border-radius: 12px; padding: 32px;
                    }
                    .login-card h1 { font-size: 16px; margin-bottom: 24px; color: var(--text); }
                    .login-card .btn { width: 100%; justify-content: center; padding: 8px 12px; font-size: 13px; }
                ")) }
            }
            body {
                div class="login-card" {
                    h1 { "WebCode" }
                    a href="/login/cognito" class="btn btn-primary" { "Sign in with Cognito" }
                }
            }
        }
    }
}

pub(crate) fn render_terminal_page(vm_id: &str, csrf_token: &str, upload_dir: &str, has_user_rootfs: bool) -> Markup {
    let short_id = format!("{}…", vm_id.get(..8).unwrap_or(vm_id));
    let terminal_script = format_terminal_script(vm_id);
    let upload_action = format!("/sessions/{vm_id}/upload");
    let file_manager_script = format_file_manager_script(csrf_token, upload_dir, &upload_action);
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "WebCode — " (short_id) }
                link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css";
                style { (PreEscaped(base_styles())) (PreEscaped("
                    body { display: flex; flex-direction: column; height: 100vh; overflow: hidden; }
                    #topbar {
                        display: flex; align-items: center; justify-content: space-between;
                        padding: 0 16px; height: 40px; flex-shrink: 0;
                        background: var(--surface); border-bottom: 1px solid var(--border);
                    }
                    #topbar-left  { display: flex; align-items: center; gap: 12px; }
                    #topbar-right { display: flex; align-items: center; gap: 8px; }
                    #vm-id { color: var(--text-muted); font-size: 12px; }
                    #main-area { display: flex; flex: 1; min-height: 0; }
                    #term-container { flex: 1; min-height: 0; background: #000; }
                    #files-panel {
                        display: none; width: 260px; flex-shrink: 0;
                        background: var(--surface); border-left: 1px solid var(--border);
                        flex-direction: column; overflow: hidden;
                    }
                    #files-panel.open { display: flex; }
                    #files-header {
                        display: flex; align-items: center; justify-content: space-between;
                        padding: 8px 12px; border-bottom: 1px solid var(--border); flex-shrink: 0;
                    }
                    #files-breadcrumb { font-size: 11px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
                    #files-list { flex: 1; overflow-y: auto; }
                    .file-entry {
                        display: flex; align-items: center; gap: 8px;
                        padding: 6px 12px; cursor: pointer;
                        border-bottom: 1px solid var(--border); font-size: 12px;
                    }
                    .file-entry:hover { background: var(--surface2); }
                    .file-entry-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
                    .file-entry-size { color: var(--text-muted); font-size: 11px; white-space: nowrap; }
                    .file-entry-dl { color: var(--text-muted); font-size: 11px; white-space: nowrap; opacity: 0; padding: 0 2px; }
                    .file-entry:hover .file-entry-dl { opacity: 1; }
                    #files-footer {
                        padding: 8px 12px; border-top: 1px solid var(--border);
                        display: flex; gap: 8px; align-items: center; flex-shrink: 0;
                    }
                    #files-upload-status { font-size: 11px; }
                    #files-upload-status.ok  { color: var(--success); }
                    #files-upload-status.err { color: var(--danger); }
                ")) }
            }
            body {
                (render_terminal_topbar(&short_id, csrf_token, has_user_rootfs))
                div id="main-area" {
                    div id="term-container" {}
                    (render_files_panel())
                }
                script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js" {}
                script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js" {}
                script { (PreEscaped(terminal_script)) }
                script { (PreEscaped(file_manager_script)) }
            }
        }
    }
}

fn render_terminal_topbar(short_id: &str, csrf_token: &str, has_user_rootfs: bool) -> Markup {
    html! {
        div id="topbar" {
            div id="topbar-left" {
                span class="topbar-title" style="font-size:14px;font-weight:600" { "WebCode" }
                span id="vm-id" { (short_id) }
            }
            div id="topbar-right" {
                @if has_user_rootfs {
                    form method="post" action="/rootfs/delete" {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        button type="submit" class="btn btn-ghost" { "Reset" }
                    }
                }
                button class="btn" onclick="toggleFiles()" { "📁 Files" }
                a href="/logout" class="btn btn-ghost" { "Logout" }
            }
        }
    }
}

fn render_files_panel() -> Markup {
    html! {
        div id="files-panel" {
            div id="files-header" {
                span id="files-breadcrumb" {}
                button class="btn btn-ghost" style="padding:2px 6px" onclick="toggleFiles()" { "✕" }
            }
            div id="files-list" {}
            div id="files-footer" {
                input type="file" id="fm-file-input" style="display:none";
                label for="fm-file-input" class="btn" style="flex:1;justify-content:center" { "↑ Upload here" }
                span id="files-upload-status" {}
            }
        }
    }
}

fn format_file_manager_script(csrf_token: &str, upload_dir: &str, upload_action: &str) -> String {
    format!(
        r#"const fmCsrfToken = "{csrf_token}";
const fmUploadDir = "{upload_dir}";
const fmUploadAction = "{upload_action}";
let fmCurrentPath = fmUploadDir;
let fmOpened = false;

function toggleFiles() {{
  const panel = document.getElementById('files-panel');
  panel.classList.toggle('open');
  if (panel.classList.contains('open') && !fmOpened) {{
    fmOpened = true;
    loadDir(fmCurrentPath);
  }}
}}

function loadDir(path) {{
  fetch('/sessions/' + vmId + '/ls?path=' + encodeURIComponent(path))
    .then(function(res) {{
      if (!res.ok) return res.text().then(function(msg) {{ throw new Error(msg); }});
      return res.json();
    }})
    .then(function(data) {{
      fmCurrentPath = path;
      renderEntries(path, data.entries);
    }})
    .catch(function(err) {{
      document.getElementById('files-list').textContent = err.message || 'Error loading directory.';
    }});
}}

function renderEntries(path, entries) {{
  document.getElementById('files-breadcrumb').textContent = path;
  const list = document.getElementById('files-list');
  list.innerHTML = '';
  if (path !== fmUploadDir) {{
    const upRow = document.createElement('div');
    upRow.className = 'file-entry';
    upRow.innerHTML = '<span>📁</span><span class="file-entry-name">..</span>';
    upRow.onclick = function() {{ loadDir(parentPath(path)); }};
    list.appendChild(upRow);
  }}
  entries.forEach(function(entry) {{
    const row = document.createElement('div');
    row.className = 'file-entry';
    const entryPath = path.replace(/\/$/, '') + '/' + entry.name;
    if (entry.is_dir) {{
      row.innerHTML = '<span>📁</span><span class="file-entry-name">' + escHtml(entry.name) + '</span><span class="file-entry-dl" title="Download as zip">↓</span>';
      row.onclick = function() {{ loadDir(entryPath); }};
      row.querySelector('.file-entry-dl').onclick = function(e) {{
        e.stopPropagation();
        window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank');
      }};
    }} else {{
      row.innerHTML = '<span>📄</span><span class="file-entry-name">' + escHtml(entry.name) + '</span><span class="file-entry-size">' + escHtml(formatSize(entry.size)) + '</span>';
      row.onclick = function() {{
        window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank');
      }};
    }}
    list.appendChild(row);
  }});
}}

function parentPath(path) {{
  const stripped = path.replace(/\/$/, '');
  const idx = stripped.lastIndexOf('/');
  if (idx <= 0) return '/';
  const parent = stripped.substring(0, idx);
  if (parent.length < fmUploadDir.length) return fmUploadDir;
  return parent;
}}

function formatSize(n) {{
  if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
  return n + ' B';
}}

function escHtml(s) {{
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}}

document.getElementById('fm-file-input').addEventListener('change', function() {{
  if (!this.files[0]) return;
  const file = this.files[0];
  const remotePath = fmCurrentPath.replace(/\/$/, '') + '/' + file.name;
  const status = document.getElementById('files-upload-status');
  status.className = '';
  status.textContent = 'Uploading…';
  const formData = new FormData();
  formData.append('csrf_token', fmCsrfToken);
  formData.append('path', remotePath);
  formData.append('file', file);
  fetch(fmUploadAction, {{ method: 'POST', body: formData }})
    .then(function(res) {{
      status.className = res.ok ? 'ok' : 'err';
      status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
      if (res.ok) loadDir(fmCurrentPath);
    }})
    .catch(function() {{
      status.className = 'err';
      status.textContent = 'Network error.';
    }})
    .finally(function() {{
      setTimeout(function() {{ status.textContent = ''; status.className = ''; }}, 3000);
    }});
  this.value = '';
}});
"#
    )
}

fn format_terminal_script(vm_id: &str) -> String {
    format!(
        r#"const vmId = "{vm_id}";
const term = new Terminal({{ cursorBlink: true, theme: {{ background: '#000000' }} }});
const fitAddon = new FitAddon.FitAddon();
term.loadAddon(fitAddon);
const container = document.getElementById('term-container');
term.open(container);
fitAddon.fit();
const ws = new WebSocket((location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host + '/ws/' + vmId);
ws.binaryType = 'arraybuffer';
function sendResize() {{
  if (ws.readyState === WebSocket.OPEN)
    ws.send(JSON.stringify({{ type: 'resize', rows: term.rows, cols: term.cols }}));
}}
term.onResize(sendResize);
ws.onopen = () => {{ term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); }};
ws.onmessage = e => term.write(new Uint8Array(e.data));
ws.onclose = () => term.write('\r\n\x1b[2mconnection closed\x1b[0m\r\n');
new ResizeObserver(() => fitAddon.fit()).observe(container);
"#
    )
}
