use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use bytes::Bytes;
use futures::{channel::mpsc, SinkExt, Stream};
use russh::client::Handle;
use russh_sftp::client::{fs::File as SftpFile, SftpSession};
use serde::Deserialize;
use std::{
    io,
    path::Path as StdPath,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use store::upsert_user;
use tokio::sync::mpsc as tokio_mpsc;
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::{
    auth::User,
    ssh::{connect_ssh, open_sftp_session, SshClient},
    state::{find_vm_guest_ip_for_user, AppError, AppState},
};

const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024; // 100 MB
const MAX_ZIP_DEPTH: usize = 10;

#[derive(Deserialize)]
pub(crate) struct DownloadQuery {
    path: String,
}

pub(crate) async fn download_file_handler(
    user: User,
    Path(vm_id): Path<String>,
    Query(query): Query<DownloadQuery>,
    State(state): State<AppState>,
) -> Result<Response<Body>, AppError> {
    if Uuid::parse_str(&vm_id).is_err() {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let guest_ip = match find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id) {
        Some(ip) => ip,
        None => return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response()),
    };
    let mut ssh_handle = connect_ssh(
        &guest_ip,
        &state.ssh_key_path,
        &state.ssh_user,
        &state.vm_host_key_path,
    )
    .await?;
    let sftp = open_sftp_session(&mut ssh_handle).await?;
    let real_path = sftp
        .canonicalize(&query.path)
        .await
        .context("failed to resolve remote path")?;
    validate_within_dir(&real_path, &state.upload_dir)?;
    let metadata = sftp
        .symlink_metadata(&real_path)
        .await
        .context("failed to stat remote path")?;
    if metadata.is_dir() {
        let dirname = extract_filename(&real_path).to_string();
        Ok(build_streaming_zip_response(
            ssh_handle,
            sftp,
            real_path,
            state.upload_dir.clone(),
            &format!("{dirname}.zip"),
        ))
    } else {
        let filename = extract_filename(&real_path).to_string();
        build_streaming_file_response(ssh_handle, sftp, &real_path, &filename).await
    }
}

// ── Single-file streaming ─────────────────────────────────────────────────────

async fn build_streaming_file_response(
    ssh_handle: Handle<SshClient>,
    sftp: SftpSession,
    path: &str,
    filename: &str,
) -> Result<Response<Body>, AppError> {
    let file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let stream = SftpFileStream {
        inner: ReaderStream::new(file),
        _sftp: sftp,
        _ssh_handle: ssh_handle,
    };
    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from_stream(stream))
        .context("failed to build response")?;
    Ok(response)
}

struct SftpFileStream {
    inner: ReaderStream<SftpFile>,
    _sftp: SftpSession,
    _ssh_handle: Handle<SshClient>,
}

impl Stream for SftpFileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

// ── Directory zip streaming ───────────────────────────────────────────────────
//
// Backpressure chain when the client is slow:
//   HTTP body stalls → zip_tx channel fills (cap 8) → spawn_blocking blocks on send
//   → file_tx channel fills (cap 4) → async SFTP reader blocks on send → SFTP reads pause.
//
// Memory: O(largest single compressed file) — only the current file's bytes are buffered
// inside SeekableChannelWriter; they are flushed to zip_tx as soon as zip finalises each
// file entry (at the forward seek that follows the local-header update).

fn build_streaming_zip_response(
    ssh_handle: Handle<SshClient>,
    sftp: SftpSession,
    dir_path: String,
    upload_dir: String,
    filename: &str,
) -> Response<Body> {
    // Channel carrying ready zip bytes to the HTTP body (bounded → backpressure).
    let (zip_tx, zip_rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    // Channel carrying (entry_name, raw_data) from the async SFTP reader to the blocking
    // zip writer (bounded → limits how far ahead SFTP reads run).
    let (file_tx, file_rx) = tokio_mpsc::channel::<(String, Vec<u8>)>(4);

    tokio::spawn(collect_zip_files(ssh_handle, sftp, dir_path, upload_dir, file_tx));
    tokio::task::spawn_blocking(move || write_zip_to_channel(file_rx, zip_tx));

    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from_stream(zip_rx))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Async task: walks the SFTP directory tree and sends `(zip_entry_name, raw_bytes)` pairs.
async fn collect_zip_files(
    _ssh_handle: Handle<SshClient>,
    sftp: SftpSession,
    dir_path: String,
    upload_dir: String,
    file_tx: tokio_mpsc::Sender<(String, Vec<u8>)>,
) {
    let mut total_bytes: usize = 0;
    let mut dirs_to_visit: Vec<(String, usize)> = vec![(dir_path.clone(), 0)];
    while let Some((dir, depth)) = dirs_to_visit.pop() {
        let read_dir = match sftp.read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in read_dir {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child_path = format!("{}/{}", dir.trim_end_matches('/'), name);
            if entry.file_type().is_symlink() {
                continue;
            }
            if validate_within_dir(&child_path, &upload_dir).is_err() {
                continue;
            }
            if entry.file_type().is_dir() {
                if depth + 1 < MAX_ZIP_DEPTH {
                    dirs_to_visit.push((child_path, depth + 1));
                }
                continue;
            }
            let data = match read_file_buffered(&sftp, &child_path).await {
                Ok(data) => data,
                Err(_) => return,
            };
            total_bytes += data.len();
            if total_bytes > MAX_DOWNLOAD_BYTES {
                return;
            }
            let relative = child_path
                .strip_prefix(&dir_path)
                .unwrap_or(&child_path)
                .trim_start_matches('/')
                .to_owned();
            if file_tx.send((relative, data)).await.is_err() {
                return;
            }
        }
    }
}

/// Blocking task: receives file data from the async reader and builds the zip using a
/// `SeekableChannelWriter`, which streams each file's compressed bytes to `zip_tx` as
/// soon as zip finalises the local entry (at the forward-seek after updating its header).
fn write_zip_to_channel(
    mut file_rx: tokio_mpsc::Receiver<(String, Vec<u8>)>,
    zip_tx: mpsc::Sender<Result<Bytes, io::Error>>,
) {
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut writer = SeekableChannelWriter::new(zip_tx);
    let zip_ok = {
        let mut zip = zip::ZipWriter::new(&mut writer);
        let mut ok = true;
        while let Some((name, data)) = file_rx.blocking_recv() {
            if zip.start_file(&name, options).is_err()
                || io::Write::write_all(&mut zip, &data).is_err()
            {
                ok = false;
                break;
            }
        }
        ok && zip.finish().is_ok()
        // zip is dropped here, releasing the &mut writer borrow
    };
    if zip_ok {
        let _ = writer.flush_remaining();
    }
}

// ── SeekableChannelWriter ─────────────────────────────────────────────────────
//
// The zip crate requires Write + Seek.  It uses seeks in exactly one pattern per file:
//   1. Write local header (unknowing CRC/sizes yet)  → pos advances to header_end
//   2. Write compressed data                         → pos advances to file_end (= high_water)
//   3. Seek BACK to header_start                     → pos drops below high_water
//   4. Overwrite header with real CRC/sizes          → pos back at header_end
//   5. Seek FORWARD to file_end                      → pos == high_water → flush
//
// At step 5 pos reaches high_water again: everything in the buffer is finalised for
// that entry and can be sent to the channel.  The buffer then drains to zero and
// the cycle repeats for the next file.

struct SeekableChannelWriter {
    buf: Vec<u8>,       // unflushed bytes; buf[0] corresponds to logical offset `base`
    base: u64,          // logical offset of buf[0]
    pos: u64,           // current write/seek position
    high_water: u64,    // highest position ever written
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
}

impl SeekableChannelWriter {
    fn new(tx: mpsc::Sender<Result<Bytes, io::Error>>) -> Self {
        Self { buf: Vec::new(), base: 0, pos: 0, high_water: 0, tx }
    }

    /// Flush `buf[0..high_water-base]` to the channel and remove those bytes from the buffer.
    fn flush_to_high_water(&mut self) -> io::Result<()> {
        let flush_len = (self.high_water - self.base) as usize;
        if flush_len == 0 || flush_len > self.buf.len() {
            return Ok(());
        }
        let chunk = Bytes::copy_from_slice(&self.buf[..flush_len]);
        futures::executor::block_on(self.tx.send(Ok(chunk)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "zip receiver dropped"))?;
        self.buf.drain(..flush_len);
        self.base = self.high_water;
        Ok(())
    }

    /// Flush all remaining buffered bytes after `zip.finish()`.
    fn flush_remaining(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::copy_from_slice(&self.buf);
        futures::executor::block_on(self.tx.send(Ok(chunk)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "zip receiver dropped"))
    }
}

impl io::Write for SeekableChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let start = (self.pos - self.base) as usize;
        let end = start + data.len();
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.buf[start..end].copy_from_slice(data);
        self.pos += data.len() as u64;
        self.high_water = self.high_water.max(self.pos);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Seek for SeekableChannelWriter {
    fn seek(&mut self, from: io::SeekFrom) -> io::Result<u64> {
        let new_pos: u64 = match from {
            io::SeekFrom::Start(p) => p,
            io::SeekFrom::Current(off) => (self.pos as i64 + off) as u64,
            io::SeekFrom::End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "seek from end not supported",
                ));
            }
        };
        if new_pos < self.base {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot seek before already-flushed data",
            ));
        }
        // Step 5 in the per-file pattern: zip seeks forward back to high_water after
        // overwriting the local header.  Flush all buffered bytes for this entry.
        if new_pos >= self.high_water {
            self.flush_to_high_water()?;
        }
        self.pos = new_pos;
        Ok(self.pos)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn read_file_buffered(sftp: &SftpSession, path: &str) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .context("failed to read remote file")?;
    Ok(buf)
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("download")
}

pub(crate) fn validate_within_dir(real_path: &str, allowed_dir: &str) -> Result<()> {
    if !StdPath::new(real_path).starts_with(allowed_dir) {
        anyhow::bail!("path is outside the allowed directory");
    }
    Ok(())
}
