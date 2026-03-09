use std::time::{SystemTime, UNIX_EPOCH};

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::state::VmInfo;

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
                title { "vm-terminal" }
                style { (PreEscaped(base_styles())) (PreEscaped("
                    body { display: flex; align-items: center; justify-content: center; min-height: 100vh; }
                    .login-card {
                        width: 320px; background: var(--surface); border: 1px solid var(--border);
                        border-radius: 12px; padding: 32px;
                    }
                    .login-card h1 { font-size: 16px; margin-bottom: 24px; color: var(--text); }
                    .login-card .btn { width: 100%; justify-content: center; padding: 8px 12px; font-size: 13px; }
                    .divider { display: flex; align-items: center; gap: 12px; margin: 16px 0; color: var(--text-muted); font-size: 11px; }
                    .divider::before, .divider::after { content: ''; flex: 1; height: 1px; background: var(--border); }
                ")) }
            }
            body {
                div class="login-card" {
                    h1 { "vm-terminal" }
                    a href="/login/cognito" class="btn btn-primary" { "Sign in with Cognito" }
                    div class="divider" { "or" }
                    a href="/demo" class="btn" { "Try Demo" }
                }
            }
        }
    }
}

pub(crate) fn render_vms_page(vms: &[VmInfo], csrf_token: &str, has_user_rootfs: bool) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "vm-terminal" }
                style { (PreEscaped(base_styles())) (PreEscaped("
                    .topbar {
                        display: flex; align-items: center; justify-content: space-between;
                        padding: 12px 24px; border-bottom: 1px solid var(--border);
                        background: var(--surface);
                    }
                    .topbar-title { font-size: 14px; font-weight: 600; }
                    .content { max-width: 900px; margin: 0 auto; padding: 24px; }
                    .section { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); overflow: hidden; }
                    .section + .section { margin-top: 16px; }
                    .section-header {
                        display: flex; align-items: center; justify-content: space-between;
                        padding: 10px 16px; border-bottom: 1px solid var(--border);
                        font-size: 12px; color: var(--text-muted); font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em;
                    }
                    table { width: 100%; border-collapse: collapse; }
                    th { padding: 8px 16px; text-align: left; font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; font-weight: 600; }
                    td { padding: 10px 16px; border-top: 1px solid var(--border); font-size: 13px; }
                    tr:hover td { background: var(--surface2); }
                    .actions { display: flex; gap: 8px; justify-content: flex-end; }
                    .empty { padding: 40px 16px; text-align: center; color: var(--text-muted); font-size: 13px; }
                    .disk-row { display: flex; align-items: center; gap: 10px; padding: 10px 16px; font-size: 12px; }
                    .disk-label { color: var(--text-muted); }
                ")) }
            }
            body {
                div class="topbar" {
                    span class="topbar-title" { "vm-terminal" }
                    form method="post" action="/vms" {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        button type="submit" class="btn btn-primary" { "+ New VM" }
                    }
                }
                div class="content" {
                    (render_vm_section(vms, csrf_token))
                    (render_disk_section(csrf_token, has_user_rootfs))
                }
            }
        }
    }
}

fn render_vm_section(vms: &[VmInfo], csrf_token: &str) -> Markup {
    html! {
        div class="section" {
            div class="section-header" {
                span { "Virtual machines" }
                span { (vms.len()) " running" }
            }
            @if vms.is_empty() {
                div class="empty" { "No running VMs. Create one to get started." }
            } @else {
                table {
                    thead {
                        tr {
                            th { "ID" }
                            th { "IP address" }
                            th { "PID" }
                            th { "Started" }
                            th {}
                        }
                    }
                    tbody {
                        @for vm in vms {
                            (render_vm_row(vm, csrf_token))
                        }
                    }
                }
            }
        }
    }
}

fn render_vm_row(vm: &VmInfo, csrf_token: &str) -> Markup {
    let short_id = format!("{}…", vm.id.get(..8).unwrap_or(&vm.id));
    let started = format_time_ago(vm.created_at);
    html! {
        tr {
            td title=(vm.id) { (short_id) }
            td style="color:var(--text-muted)" { (vm.guest_ip) }
            td style="color:var(--text-muted)" { (vm.pid) }
            td style="color:var(--text-muted)" { (started) }
            td {
                div class="actions" {
                    a href={ "/terminal/" (vm.id) } class="btn" { "Connect" }
                    form method="post" action={ "/vms/" (vm.id) "/delete" } {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        button type="submit" class="btn btn-danger" { "Delete" }
                    }
                }
            }
        }
    }
}

fn render_disk_section(csrf_token: &str, has_user_rootfs: bool) -> Markup {
    html! {
        div class="section" {
            div class="section-header" { span { "Saved disk" } }
            div class="disk-row" {
                span class="disk-label" { "Status:" }
                @if has_user_rootfs {
                    span class="badge badge-green" { "saved" }
                    span style="color:var(--text-muted)" { "Your disk will be restored on the next VM." }
                    form method="post" action="/rootfs/delete" style="margin-left:auto" {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        button type="submit" class="btn btn-danger" { "Delete saved disk" }
                    }
                } @else {
                    span class="badge badge-gray" { "none" }
                    span style="color:var(--text-muted)" { "Base image will be used on next VM." }
                }
            }
        }
    }
}

pub(crate) fn render_terminal_page(vm_id: &str, csrf_token: &str, upload_dir: &str) -> Markup {
    let short_id = format!("{}…", vm_id.get(..8).unwrap_or(vm_id));
    let terminal_script = format_terminal_script(vm_id);
    let upload_action = format!("/vms/{vm_id}/upload");
    let default_path = format!("{upload_dir}/");
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "vm-terminal — " (short_id) }
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
                    #upload-drawer {
                        display: none; align-items: center; gap: 10px;
                        padding: 0 16px; height: 44px; flex-shrink: 0;
                        background: var(--surface2); border-bottom: 1px solid var(--border);
                    }
                    #upload-drawer.open { display: flex; }
                    #file-label {
                        display: inline-flex; align-items: center; gap: 6px;
                        padding: 4px 10px; border-radius: var(--radius);
                        border: 1px solid var(--border); background: var(--surface);
                        color: var(--text); font-size: 12px; cursor: pointer; white-space: nowrap;
                    }
                    #file-label:hover { border-color: var(--text-muted); }
                    #file-name { color: var(--text-muted); font-size: 12px; min-width: 120px; }
                    #upload-path { flex: 1; min-width: 0; }
                    #upload-status { font-size: 12px; white-space: nowrap; }
                    #upload-status.ok  { color: var(--success); }
                    #upload-status.err { color: var(--danger); }
                    #term-container { flex: 1; min-height: 0; background: #000; }
                ")) }
            }
            body {
                div id="topbar" {
                    div id="topbar-left" {
                        a href="/vms" class="btn btn-ghost" style="padding:4px 8px" { "← VMs" }
                        span id="vm-id" { (short_id) }
                    }
                    div id="topbar-right" {
                        button id="upload-toggle" class="btn" onclick="toggleUpload()" { "↑ Upload" }
                    }
                }
                form id="upload-drawer" method="post" action=(upload_action) enctype="multipart/form-data" {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    input type="file" id="file-input" name="file" style="display:none";
                    label id="file-label" for="file-input" { "📎 Choose file" }
                    span id="file-name" { "No file chosen" }
                    input id="upload-path" type="text" name="path" value=(default_path);
                    button type="submit" class="btn btn-primary" { "Upload" }
                    span id="upload-status" {}
                }
                div id="term-container" {}
                script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js" {}
                script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js" {}
                script { (PreEscaped(terminal_script)) }
                script { (PreEscaped(upload_script())) }
            }
        }
    }
}

fn upload_script() -> &'static str {
    r#"function toggleUpload() {
  document.getElementById('upload-drawer').classList.toggle('open');
}
document.getElementById('file-input').addEventListener('change', function() {
  const name = this.files[0]?.name || 'No file chosen';
  document.getElementById('file-name').textContent = name;
  if (this.files[0]) {
    const path = document.getElementById('upload-path');
    path.value = path.value.replace(/[^/]+$/, '') + name;
  }
});
document.getElementById('upload-drawer').addEventListener('submit', async function(e) {
  e.preventDefault();
  const status = document.getElementById('upload-status');
  status.className = '';
  status.textContent = 'Uploading…';
  try {
    const res = await fetch(this.action, { method: 'POST', body: new FormData(this) });
    status.className = res.ok ? 'ok' : 'err';
    status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
  } catch (_) {
    status.className = 'err';
    status.textContent = 'Network error.';
  }
  setTimeout(() => { status.textContent = ''; status.className = ''; }, 3000);
});"#
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

fn format_time_ago(created_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let diff = now.saturating_sub(created_at);
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}
