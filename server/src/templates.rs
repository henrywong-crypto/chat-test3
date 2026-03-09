use std::time::{SystemTime, UNIX_EPOCH};

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::state::VmInfo;

pub(crate) fn render_login_page() -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" data-theme="dark" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "vm-terminal — login" }
                link href="https://cdn.jsdelivr.net/npm/daisyui@latest/dist/full.css" rel="stylesheet";
                script src="https://cdn.tailwindcss.com" {}
            }
            body class="font-mono min-h-screen flex items-center justify-center bg-base-200" {
                div class="card w-80 bg-base-100 shadow-xl" {
                    div class="card-body gap-4" {
                        h1 class="card-title text-primary" { "vm-terminal" }
                        a href="/login/cognito" class="btn btn-primary w-full" { "Login with Cognito" }
                        div class="divider text-xs opacity-60" { "or" }
                        a href="/demo" class="btn btn-neutral w-full" { "Try Demo" }
                    }
                }
            }
        }
    }
}

pub(crate) fn render_vms_page(vms: &[VmInfo], csrf_token: &str, has_user_rootfs: bool) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" data-theme="dark" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "vm-terminal" }
                link href="https://cdn.jsdelivr.net/npm/daisyui@latest/dist/full.css" rel="stylesheet";
                script src="https://cdn.tailwindcss.com" {}
            }
            body class="font-mono bg-base-200 min-h-screen" {
                div class="p-6" {
                    div class="flex justify-between items-center mb-6" {
                        h1 class="text-xl font-bold text-primary" { "vm-terminal" }
                        form method="post" action="/vms" {
                            input type="hidden" name="csrf_token" value=(csrf_token);
                            button type="submit" class="btn btn-primary btn-sm" { "+ New VM" }
                        }
                    }
                    (render_vm_table(vms, csrf_token))
                    (render_disk_panel(csrf_token, has_user_rootfs))
                }
            }
        }
    }
}

fn render_vm_table(vms: &[VmInfo], csrf_token: &str) -> Markup {
    if vms.is_empty() {
        return html! {
            p class="text-base-content/50 text-center py-8" { "No running VMs." }
        };
    }
    html! {
        div class="overflow-x-auto" {
            table class="table table-zebra w-full" {
                thead {
                    tr {
                        th { "ID" }
                        th { "IP" }
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

fn render_vm_row(vm: &VmInfo, csrf_token: &str) -> Markup {
    let short_id = format!("{}…", vm.id.get(..8).unwrap_or(&vm.id));
    let started = format_time_ago(vm.created_at);
    html! {
        tr {
            td title=(vm.id) { (short_id) }
            td { (vm.guest_ip) }
            td { (vm.pid) }
            td { (started) }
            td class="flex gap-2" {
                a href={ "/terminal/" (vm.id) } class="btn btn-sm" { "Connect" }
                form method="post" action={ "/vms/" (vm.id) "/delete" } {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    button type="submit" class="btn btn-error btn-sm" { "Delete" }
                }
            }
        }
    }
}

fn render_disk_panel(csrf_token: &str, has_user_rootfs: bool) -> Markup {
    html! {
        div class="mt-6 flex items-center gap-3 p-3 bg-base-100 rounded-lg" {
            span class="text-sm text-base-content/70" { "Saved disk:" }
            @if has_user_rootfs {
                span class="badge badge-success badge-sm" { "exists" }
                form method="post" action="/rootfs/delete" {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    button type="submit" class="btn btn-ghost btn-xs text-error" { "Delete" }
                }
            } @else {
                span class="badge badge-neutral badge-sm" { "none" }
                span class="text-xs text-base-content/50" { "(base image used on next VM)" }
            }
        }
    }
}

pub(crate) fn render_terminal_page(vm_id: &str, csrf_token: &str, upload_dir: &str) -> Markup {
    let short_id = format!("{}…", vm_id.get(..8).unwrap_or(vm_id));
    let terminal_script = format_terminal_script(vm_id);
    let upload_action = format!("/vms/{vm_id}/upload");
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "vm-terminal" }
                link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css";
                style {
                    "body { margin: 0; background: #000; font-family: ui-monospace, monospace; }"
                    "#term-container { height: calc(100vh - 45px - 46px); }"
                }
            }
            body {
                div id="term-header" style="display:flex;align-items:center;gap:12px;padding:6px 12px;background:#161b22;border-bottom:1px solid #30363d" {
                    a href="/vms" style="color:#c9d1d9;text-decoration:none;font-size:13px" { "← VMs" }
                    span style="font-size:12px;color:#8b949e" { (short_id) }
                }
                form id="upload-form" method="post" action=(upload_action) enctype="multipart/form-data"
                    style="display:flex;align-items:center;gap:8px;padding:6px 12px;background:#0d1117;border-bottom:1px solid #30363d" {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    input type="file" name="file" required
                        style="font-size:12px;color:#c9d1d9;background:transparent;border:none;flex:0 0 auto";
                    input type="text" name="path" value=(format!("{upload_dir}/")) required
                        style="font-size:12px;color:#c9d1d9;background:#161b22;border:1px solid #30363d;border-radius:4px;padding:2px 6px;flex:1 1 auto";
                    button type="submit"
                        style="font-size:12px;color:#c9d1d9;background:#21262d;border:1px solid #30363d;border-radius:4px;padding:2px 10px;cursor:pointer" {
                        "Upload"
                    }
                    span id="upload-status" style="font-size:12px;color:#8b949e" {}
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
    r#"document.getElementById('upload-form').addEventListener('submit', async function(e) {
  e.preventDefault();
  const status = document.getElementById('upload-status');
  status.textContent = 'Uploading…';
  try {
    const res = await fetch(this.action, { method: 'POST', body: new FormData(this) });
    status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
  } catch (err) {
    status.textContent = 'Upload error.';
  }
  setTimeout(() => { status.textContent = ''; }, 3000);
});"#
}

fn format_terminal_script(vm_id: &str) -> String {
    format!(
        r#"const vmId = "{vm_id}";
const term = new Terminal({{ cursorBlink: true }});
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
ws.onclose = () => term.write('\r\nconnection closed\r\n');
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
