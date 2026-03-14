use anyhow::{Context, Result, bail};
use axum::{
    body::Body,
    http::{HeaderValue, Response, header},
};
use bytes::Bytes;
use futures::channel::mpsc;
use russh_sftp::client::SftpSession;
use std::{
    io::{Error as IoError, Write},
    path::PathBuf,
};
use tokio::{
    io::AsyncReadExt,
    sync::mpsc as tokio_mpsc,
    time::{Duration, timeout},
};
use zip::write::SimpleFileOptions;

use crate::seekable_channel_writer::SeekableChannelWriter;
use common::validate_within_dir;

const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024; // 100 MB
const MAX_ZIP_DEPTH: usize = 10;
const FILE_CHUNK_SIZE: usize = 64 * 1024; // 64 KB
const SEND_TIMEOUT_SECS: u64 = 30;

enum FileEvent {
    Start(String),
    Chunk(Bytes),
}

pub fn build_streaming_zip_response(
    sftp: SftpSession,
    dir_path: PathBuf,
    upload_dir: PathBuf,
    filename: &str,
) -> Result<Response<Body>> {
    // zip bytes → HTTP body (bounded for backpressure)
    let (zip_tx, zip_rx) = mpsc::channel::<Result<Bytes, IoError>>(8);
    // file events → zip writer (bounded to limit SFTP read-ahead)
    let (file_tx, file_rx) = tokio_mpsc::channel::<FileEvent>(4);

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
    dir_path: PathBuf,
    upload_dir: PathBuf,
    file_tx: tokio_mpsc::Sender<FileEvent>,
) {
    let mut total_bytes: usize = 0;
    let mut dirs_to_visit: Vec<(PathBuf, usize)> = vec![(dir_path.clone(), 0)];
    while let Some((dir, depth)) = dirs_to_visit.pop() {
        let dir_str = match dir.to_str() {
            Some(s) => s,
            None => return,
        };
        let read_dir = match sftp.read_dir(dir_str).await {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in read_dir {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child_path = dir.join(&name);
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
            let relative = match child_path
                .strip_prefix(&dir_path)
                .unwrap_or(&child_path)
                .to_str()
            {
                Some(s) => s.to_owned(),
                None => return,
            };
            match timeout(
                Duration::from_secs(SEND_TIMEOUT_SECS),
                file_tx.send(FileEvent::Start(relative)),
            )
            .await
            {
                Ok(Ok(())) => {}
                _ => return,
            }
            let child_str = match child_path.to_str() {
                Some(s) => s,
                None => return,
            };
            if stream_file(&sftp, child_str, &file_tx, &mut total_bytes)
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

async fn stream_file(
    sftp: &SftpSession,
    path: &str,
    file_tx: &tokio_mpsc::Sender<FileEvent>,
    total_bytes: &mut usize,
) -> Result<()> {
    let mut file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let mut buf = vec![0u8; FILE_CHUNK_SIZE];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .context("failed to read remote file")?;
        if n == 0 {
            break;
        }
        *total_bytes += n;
        if *total_bytes > MAX_DOWNLOAD_BYTES {
            bail!("download size limit exceeded");
        }
        match timeout(
            Duration::from_secs(SEND_TIMEOUT_SECS),
            file_tx.send(FileEvent::Chunk(Bytes::copy_from_slice(&buf[..n]))),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => bail!("zip writer dropped"),
            Err(_) => bail!("zip writer send timed out"),
        }
    }
    Ok(())
}

fn write_zip_to_channel(
    mut file_rx: tokio_mpsc::Receiver<FileEvent>,
    zip_tx: mpsc::Sender<Result<Bytes, IoError>>,
) {
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut writer = SeekableChannelWriter::new(zip_tx);
    let zip_ok = {
        let mut zip = zip::ZipWriter::new(&mut writer);
        let mut ok = true;
        while let Some(event) = file_rx.blocking_recv() {
            match event {
                FileEvent::Start(name) => {
                    if zip.start_file(&name, options).is_err() {
                        ok = false;
                        break;
                    }
                }
                FileEvent::Chunk(data) => {
                    if Write::write_all(&mut zip, &data).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
        }
        ok && zip.finish().is_ok()
    };
    if zip_ok {
        let _ = writer.flush_remaining();
    }
}
