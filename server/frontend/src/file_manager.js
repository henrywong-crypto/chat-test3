const config = document.getElementById('app-config').dataset;
const vmId = config.vmId;
const fmCsrfToken = config.csrfToken;
const fmUploadDir = config.uploadDir;
const fmUploadAction = config.uploadAction;

let fmCurrentPath = fmUploadDir;
let fmOpened = false;

document.getElementById('files-toggle-btn').addEventListener('click', toggleFiles);
document.getElementById('files-close-btn').addEventListener('click', toggleFiles);
function toggleFiles() {
  const panel = document.getElementById('files-panel');
  panel.classList.toggle('open');
  if (panel.classList.contains('open') && !fmOpened) {
    fmOpened = true;
    loadDir(fmCurrentPath);
  }
};

function loadDir(path) {
  fetch('/sessions/' + vmId + '/ls?path=' + encodeURIComponent(path))
    .then(function(res) {
      if (!res.ok) return res.text().then(function(msg) { throw new Error(msg); });
      return res.json();
    })
    .then(function(data) {
      fmCurrentPath = path;
      renderEntries(path, data.entries);
    })
    .catch(function(err) {
      document.getElementById('files-list').textContent = err.message || 'Error loading directory.';
    });
}

function renderEntries(path, entries) {
  document.getElementById('files-breadcrumb').textContent = path;
  const list = document.getElementById('files-list');
  list.innerHTML = '';
  if (path !== fmUploadDir) {
    const upRow = document.createElement('div');
    upRow.className = 'file-entry';
    upRow.innerHTML = '<span>\u{1F4C1}</span><span class="file-entry-name">..</span>';
    upRow.onclick = function() { loadDir(parentPath(path)); };
    list.appendChild(upRow);
  }
  entries.forEach(function(entry) {
    const row = document.createElement('div');
    row.className = 'file-entry';
    const entryPath = path.replace(/\/$/, '') + '/' + entry.name;
    if (entry.is_dir) {
      row.innerHTML =
        '<span>\u{1F4C1}</span>' +
        '<span class="file-entry-name">' + escHtml(entry.name) + '</span>' +
        '<span class="file-entry-dl" title="Download as zip">\u2193</span>';
      row.onclick = function() { loadDir(entryPath); };
      row.querySelector('.file-entry-dl').onclick = function(e) {
        e.stopPropagation();
        window.open('/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath), '_blank');
      };
    } else {
      row.innerHTML =
        '<span>\u{1F4C4}</span>' +
        '<span class="file-entry-name">' + escHtml(entry.name) + '</span>' +
        '<span class="file-entry-size">' + escHtml(formatSize(entry.size)) + '</span>';
      row.onclick = function() {
        window.location.href = '/sessions/' + vmId + '/download?path=' + encodeURIComponent(entryPath);
      };
    }
    list.appendChild(row);
  });
}

function parentPath(path) {
  const stripped = path.replace(/\/$/, '');
  const idx = stripped.lastIndexOf('/');
  if (idx <= 0) return '/';
  const parent = stripped.substring(0, idx);
  if (parent.length < fmUploadDir.length) return fmUploadDir;
  return parent;
}

function formatSize(n) {
  if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
  return n + ' B';
}

function escHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

document.getElementById('fm-file-input').addEventListener('change', function() {
  if (!this.files[0]) return;
  const file = this.files[0];
  const remotePath = fmCurrentPath.replace(/\/$/, '') + '/' + file.name;
  const status = document.getElementById('files-upload-status');
  status.className = '';
  status.textContent = 'Uploading\u2026';
  const formData = new FormData();
  formData.append('csrf_token', fmCsrfToken);
  formData.append('path', remotePath);
  formData.append('file', file);
  fetch(fmUploadAction, { method: 'POST', body: formData })
    .then(function(res) {
      status.className = res.ok ? 'ok' : 'err';
      status.textContent = res.ok ? 'Uploaded.' : 'Upload failed.';
      if (res.ok) loadDir(fmCurrentPath);
    })
    .catch(function() {
      status.className = 'err';
      status.textContent = 'Network error.';
    })
    .finally(function() {
      setTimeout(function() { status.textContent = ''; status.className = ''; }, 3000);
    });
  this.value = '';
});
