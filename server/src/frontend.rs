pub(crate) const FRONTEND_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <title>vm-terminal</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5/css/xterm.css" />
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    body { margin: 0; background: #0d1117; color: #c9d1d9; font-family: ui-monospace, monospace; }

    /* ── list view ── */
    #list-view { padding: 24px; }
    .list-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
    h1 { margin: 0; color: #58a6ff; font-size: 20px; }
    table { width: 100%; border-collapse: collapse; }
    th { text-align: left; padding: 8px 12px; color: #8b949e; font-size: 11px;
         text-transform: uppercase; border-bottom: 1px solid #21262d; }
    td { padding: 10px 12px; border-bottom: 1px solid #161b22; font-size: 13px; }
    tr:hover td { background: #161b22; }
    .empty { color: #8b949e; padding: 32px 0; text-align: center; }

    /* ── terminal view ── */
    #terminal-view { display: none; position: fixed; inset: 0;
                     flex-direction: column; background: #000; }
    #term-header { display: flex; align-items: center; gap: 12px; padding: 6px 12px;
                   background: #161b22; border-bottom: 1px solid #30363d; flex-shrink: 0; }
    #term-vm-id { font-size: 12px; color: #8b949e; }
    #term-container { flex: 1; min-height: 0; }

    /* ── buttons ── */
    button { background: #21262d; color: #c9d1d9; border: 1px solid #30363d;
             padding: 5px 12px; cursor: pointer; border-radius: 6px; font-size: 13px; }
    button:hover { background: #30363d; }
    .btn-primary { background: #238636; border-color: #2ea043; }
    .btn-primary:hover { background: #2ea043; }
    .btn-danger { background: #6e1b1b; border-color: #da3633; }
    .btn-danger:hover { background: #da3633; }
    .btn-primary:disabled { background: #1a4a23; border-color: #1a4a23; color: #4d7a57; cursor: default; }

    /* ── upload dialog ── */
    #upload-dialog { display: none; position: fixed; inset: 0; background: rgba(0,0,0,.6);
                     align-items: center; justify-content: center; z-index: 10; }
    #upload-dialog.open { display: flex; }
    .dialog-box { background: #161b22; border: 1px solid #30363d; border-radius: 8px;
                  padding: 20px; width: 400px; display: flex; flex-direction: column; gap: 12px; }
    .dialog-box h2 { margin: 0; font-size: 15px; color: #c9d1d9; }
    .dialog-box label { font-size: 12px; color: #8b949e; }
    .dialog-box input[type=text] { width: 100%; background: #0d1117; color: #c9d1d9;
                                    border: 1px solid #30363d; border-radius: 4px;
                                    padding: 6px 8px; font-family: inherit; font-size: 13px; }
    .dialog-actions { display: flex; justify-content: flex-end; gap: 8px; }
  </style>
</head>
<body>

<!-- List view -->
<div id="list-view">
  <div class="list-header">
    <h1>vm-terminal</h1>
    <button id="new-btn" class="btn-primary" onclick="newVm()">+ New VM</button>
  </div>
  <div id="vm-table-wrap"></div>
</div>

<!-- Terminal view -->
<div id="terminal-view">
  <div id="term-header">
    <button onclick="backToList()">&#8592; VMs</button>
    <span id="term-vm-id"></span>
  </div>
  <div id="term-container"></div>
</div>

<!-- Upload dialog -->
<div id="upload-dialog">
  <div class="dialog-box">
    <h2>Upload file to VM</h2>
    <label>Destination path</label>
    <input type="text" id="upload-path" placeholder="/home/user/uploads/file.txt" />
    <label>File</label>
    <input type="file" id="upload-file" />
    <div class="dialog-actions">
      <button onclick="closeUpload()">Cancel</button>
      <button class="btn-primary" onclick="submitUpload()">Upload</button>
    </div>
  </div>
</div>

<script src="https://cdn.jsdelivr.net/npm/xterm@5/lib/xterm.js"></script>
<script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8/lib/xterm-addon-fit.js"></script>
<script>
  let ws = null, term = null, fitAddon = null, refreshTimer = null, uploadVmId = null;

  function ago(secs) {
    const d = Math.floor(Date.now() / 1000) - secs;
    if (d < 60)    return d + 's ago';
    if (d < 3600)  return Math.floor(d / 60) + 'm ago';
    if (d < 86400) return Math.floor(d / 3600) + 'h ago';
    return Math.floor(d / 86400) + 'd ago';
  }

  async function loadVms() {
    const vms = await fetch('/vms').then(r => r.json());
    const wrap = document.getElementById('vm-table-wrap');
    if (!vms.length) {
      wrap.innerHTML = '<p class="empty">No running VMs.</p>';
      return;
    }
    wrap.innerHTML = `
      <table>
        <thead><tr><th>ID</th><th>IP</th><th>PID</th><th>Started</th><th></th></tr></thead>
        <tbody>${vms.map(v => `
          <tr>
            <td title="${v.id}">${v.id.slice(0, 8)}&hellip;</td>
            <td>${v.guest_ip}</td>
            <td>${v.pid}</td>
            <td>${ago(v.created_at)}</td>
            <td style="display:flex;gap:6px">
              <button onclick="connectVm('${v.id}')">Connect</button>
              <button onclick="openUpload('${v.id}')">Upload</button>
              <button class="btn-danger" onclick="deleteVm('${v.id}')">Delete</button>
            </td>
          </tr>`).join('')}
        </tbody>
      </table>`;
  }

  function startRefresh() { loadVms(); refreshTimer = setInterval(loadVms, 5000); }
  function stopRefresh()  { clearInterval(refreshTimer); refreshTimer = null; }

  function showTerminal(label) {
    stopRefresh();
    document.getElementById('list-view').style.display = 'none';
    const tv = document.getElementById('terminal-view');
    tv.style.display = 'flex';
    document.getElementById('term-vm-id').textContent = label;
  }

  function backToList() {
    if (ws)       { ws.close(); ws = null; }
    if (term)     { term.dispose(); term = null; fitAddon = null; }
    document.getElementById('term-container').innerHTML = '';
    document.getElementById('terminal-view').style.display = 'none';
    document.getElementById('list-view').style.display = '';
    startRefresh();
  }

  function openTerminal(vmId) {
    showTerminal(vmId.slice(0, 8) + '\u2026');
    term = new Terminal({ cursorBlink: true });
    fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    const container = document.getElementById('term-container');
    term.open(container);

    ws = new WebSocket('ws://' + location.host + '/ws/' + vmId);
    ws.binaryType = 'arraybuffer';

    function sendResize() {
      if (ws.readyState === WebSocket.OPEN)
        ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
    }
    term.onResize(sendResize);
    ws.onopen = () => { term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); };
    ws.onmessage = e => term.write(new Uint8Array(e.data));
    ws.onclose   = () => term.write('\r\nconnection closed\r\n');
    new ResizeObserver(() => fitAddon.fit()).observe(container);
  }

  async function newVm() {
    const btn = document.getElementById('new-btn');
    btn.disabled = true;
    btn.textContent = 'Starting\u2026';
    try {
      const res = await fetch('/vms', { method: 'POST' });
      if (!res.ok) { alert('Failed to create VM'); return; }
      const vm = await res.json();
      openTerminal(vm.id);
    } finally {
      btn.disabled = false;
      btn.textContent = '+ New VM';
    }
  }

  function connectVm(id) { openTerminal(id); }

  async function deleteVm(id) {
    await fetch('/vms/' + id, { method: 'DELETE' });
    loadVms();
  }

  function openUpload(vmId) {
    uploadVmId = vmId;
    document.getElementById('upload-path').value = '';
    document.getElementById('upload-file').value = '';
    document.getElementById('upload-dialog').classList.add('open');
  }

  function closeUpload() {
    uploadVmId = null;
    document.getElementById('upload-dialog').classList.remove('open');
  }

  async function submitUpload() {
    const path = document.getElementById('upload-path').value.trim();
    const fileInput = document.getElementById('upload-file');
    if (!path || !fileInput.files.length) { alert('Path and file are required'); return; }
    const form = new FormData();
    form.append('path', path);
    form.append('file', fileInput.files[0]);
    const res = await fetch('/vms/' + uploadVmId + '/upload', { method: 'POST', body: form });
    if (res.ok) {
      closeUpload();
    } else {
      alert('Upload failed: ' + await res.text());
    }
  }

  startRefresh();
</script>
</body>
</html>"#;
