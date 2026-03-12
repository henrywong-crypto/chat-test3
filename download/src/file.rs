use anyhow::Context;
use axum::{
    body::Body,
    http::{header, HeaderValue, Response},
};
use bytes::Bytes;
use futures::Stream;
use russh_sftp::client::{fs::File as SftpFile, SftpSession};
use std::{
    io,
    path::Path,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio_util::io::ReaderStream;

pub async fn build_streaming_file_response(
    sftp: SftpSession,
    path: &Path,
) -> anyhow::Result<Response<Body>> {
    let path_str = path.to_str().context("remote path must be valid UTF-8")?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let file = sftp
        .open(path_str)
        .await
        .context("failed to open remote file")?;
    let stream = SftpFileStream {
        inner: ReaderStream::new(file),
        _sftp: sftp,
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
    // Held to keep the SSH channel open until the stream is fully consumed.
    _sftp: SftpSession,
}

impl Stream for SftpFileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
