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
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio_util::io::ReaderStream;

pub async fn build_streaming_file_response(
    sftp: SftpSession,
    path: &str,
    filename: &str,
) -> anyhow::Result<Response<Body>> {
    let file = sftp
        .open(path)
        .await
        .context("failed to open remote file")?;
    let stream = SftpFileStream {
        inner: ReaderStream::new(file),
        sftp,
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
    sftp: SftpSession,
}

impl Stream for SftpFileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
