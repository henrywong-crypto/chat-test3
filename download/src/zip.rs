use anyhow::Context;
use axum::{
    body::Body,
    http::{header, HeaderValue, Response},
};
use bytes::Bytes;
use futures::{channel::mpsc, SinkExt};
use russh_sftp::client::SftpSession;
use std::io;
use tokio::sync::mpsc as tokio_mpsc;
use zip::write::SimpleFileOptions;

use crate::validate_within_dir;

const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024; // 100 MB
const MAX_ZIP_DEPTH: usize = 10;

pub fn build_streaming_zip_response(
    sftp: SftpSession,
    dir_path: String,
    upload_dir: String,
    filename: &str,
) -> anyhow::Result<Response<Body>> {
    // zip bytes → HTTP body (bounded for backpressure)
    let (zip_tx, zip_rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    // file data → zip writer (bounded to limit SFTP read-ahead)
    let (file_tx, file_rx) = tokio_mpsc::channel::<(String, Vec<u8>)>(4);

    tokio::spawn(collect_zip_files(sftp, dir_path, upload_dir, file_tx));
    tokio::task::spawn_blocking(move || write_zip_to_channel(file_rx, zip_tx));

    let content_disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .context("failed to build content disposition header")?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from_stream(zip_rx))
        .context("failed to build zip response")
}

async fn collect_zip_files(
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

fn write_zip_to_channel(
    mut file_rx: tokio_mpsc::Receiver<(String, Vec<u8>)>,
    zip_tx: mpsc::Sender<Result<Bytes, io::Error>>,
) {
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
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
    };
    if zip_ok {
        let _ = writer.flush_remaining();
    }
}

async fn read_file_buffered(sftp: &SftpSession, path: &str) -> anyhow::Result<Vec<u8>> {
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

// ── SeekableChannelWriter ─────────────────────────────────────────────────────
//
// zip 2.x requires Write + Seek. For each file it follows this pattern:
//   1. Write local header (pos → header_end)
//   2. Write compressed data (pos → file_end = high_water)
//   3. Seek BACK to header_start to overwrite with real CRC/sizes
//   4. Write updated header (pos → header_end again)
//   5. Seek FORWARD to file_end (pos == high_water) → flush all buffered bytes
//
// At step 5 every byte for that entry is finalised and can be sent to the channel.
// The buffer drains to zero and the cycle repeats, so memory stays O(largest file).

struct SeekableChannelWriter {
    buf: Vec<u8>,    // unflushed bytes; buf[0] is at logical offset `base`
    base: u64,       // logical stream offset of buf[0]
    pos: u64,        // current read/write position
    high_water: u64, // highest position ever reached
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
}

impl SeekableChannelWriter {
    fn new(tx: mpsc::Sender<Result<Bytes, io::Error>>) -> Self {
        Self {
            buf: Vec::new(),
            base: 0,
            pos: 0,
            high_water: 0,
            tx,
        }
    }

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
        // Step 5: zip seeks forward back to high_water after rewriting the local header.
        // Everything buffered up to high_water is now final — flush it.
        if new_pos >= self.high_water {
            self.flush_to_high_water()?;
        }
        self.pos = new_pos;
        Ok(self.pos)
    }
}
