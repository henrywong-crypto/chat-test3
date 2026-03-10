import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

const config = document.getElementById('app-config').dataset;
const vmId = config.vmId;
const fmCsrfToken = config.csrfToken;
const fmUploadDir = config.uploadDir;
const fmUploadAction = config.uploadAction;

// ── Terminal ──────────────────────────────────────────────────────────────────

const term = new Terminal({ cursorBlink: true, theme: { background: '#000000' } });
const fitAddon = new FitAddon();
term.loadAddon(fitAddon);

const container = document.getElementById('term-container');
term.open(container);
fitAddon.fit();
term.focus();

const ws = new WebSocket(
  (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host + '/ws/' + vmId
);
ws.binaryType = 'arraybuffer';

function sendResize() {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
  }
}

term.onResize(sendResize);
ws.onopen = () => { term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); };
ws.onmessage = e => term.write(new Uint8Array(e.data));
ws.onclose = () => term.write('\r\n\x1b[2mconnection closed\x1b[0m\r\n');
new ResizeObserver(() => fitAddon.fit()).observe(container);

// ── File manager ──────────────────────────────────────────────────────────────

let fmCurrentPath = fmUploadDir;
let fmOpened = false;

document.getElementById('files-toggle-btn').addEventListener('click', toggleFiles);
document.getElementById('files-close-btn').addEventListener('click', toggleFiles);

function toggleFiles() {
  const panel = document.getElementById('files-panel');
  const isOpen = panel.classList.toggle('flex');
  panel.classList.toggle('hidden', !isOpen);
  if (isOpen && !fmOpened) {
    fmOpened = true;
    loadDir(fmCurrentPath);
  }
}

function loadDir(path) {
  fetch('/sessions/' + vmId + '/ls?path=' + encodeURIComponent(path))
    .then(res => res.ok ? res.json() : res.text().then(msg => { throw new Error(msg); }))
    .then(data => { fmCurrentPath = path; renderEntries(path, data.entries); })
    .catch(err => { document.getElementById('files-list').textContent = err.message || 'Error.'; });
}

function renderEntries(path, entries) {
  document.getElementById('files-breadcrumb').textContent = path;
  const list = document.getElementById('files-list');
  list.innerHTML = '';
  if (path !== fmUploadDir) {
    list.appendChild(buildEntryRow('📁', '..', () => loadDir(parentPath(path))));
  }
  for (const entry of entries) {
    const entryPath = path.replace(/\/$/, '') + '/' + entry.name;
    if (entry.is_dir) {
      const row = buildEntryRow('📁', entry.name, () => loadDir(entryPath));
      const dl = document.createElement('span');
      dl.className = 'text-xs opacity-0 group-hover:opacity-100 px-1 cursor-pointer';
      dl.title = 'Download as zip';
      dl.textContent = '↓';
      dl.onclick = e => { e.stopPropagation(); window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank'); };
      row.appendChild(dl);
      list.appendChild(row);
    } else {
      const row = buildEntryRow('📄', entry.name, () => { window.location.href = '/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath); });
      const size = document.createElement('span');
      size.className = 'text-xs opacity-50 whitespace-nowrap';
      size.textContent = formatSize(entry.size);
      row.appendChild(size);
      list.appendChild(row);
    }
  }
}

function buildEntryRow(icon, name, onclick) {
  const row = document.createElement('div');
  row.className = 'group flex items-center gap-2 px-3 py-1.5 cursor-pointer border-b border-base-300 text-xs hover:bg-base-300';
  row.onclick = onclick;
  const iconEl = document.createElement('span');
  iconEl.textContent = icon;
  const nameEl = document.createElement('span');
  nameEl.className = 'flex-1 truncate';
  nameEl.textContent = name;
  row.appendChild(iconEl);
  row.appendChild(nameEl);
  return row;
}

function parentPath(path) {
  const stripped = path.replace(/\/$/, '');
  const idx = stripped.lastIndexOf('/');
  const parent = idx <= 0 ? '/' : stripped.substring(0, idx);
  return parent.length < fmUploadDir.length ? fmUploadDir : parent;
}

function formatSize(n) {
  if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
  return n + ' B';
}

document.getElementById('fm-file-input').addEventListener('change', function() {
  if (!this.files[0]) return;
  const file = this.files[0];
  const status = document.getElementById('files-upload-status');
  status.className = 'text-xs';
  status.textContent = 'Uploading…';
  const formData = new FormData();
  formData.append('csrf_token', fmCsrfToken);
  formData.append('path', fmCurrentPath.replace(/\/$/, '') + '/' + file.name);
  formData.append('file', file);
  fetch(fmUploadAction, { method: 'POST', body: formData })
    .then(res => {
      status.className = 'text-xs ' + (res.ok ? 'text-success' : 'text-error');
      status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
      if (res.ok) loadDir(fmCurrentPath);
    })
    .catch(() => { status.className = 'text-xs text-error'; status.textContent = 'Network error.'; })
    .finally(() => setTimeout(() => { status.textContent = ''; status.className = 'text-xs'; }, 3000));
  this.value = '';
});
