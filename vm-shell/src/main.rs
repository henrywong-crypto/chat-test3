//! vm-shell: runs inside the Firecracker VM.
//!
//! Listens on vsock port 5000. For each connection from the host it opens a
//! PTY, spawns bash, and bridges the PTY master ↔ vsock using a simple frame
//! protocol:
//!
//!   [type: u8][len: u32 LE][payload: len bytes]
//!
//!   MSG_OPEN   0x00  host→vm  first frame: [cols u16 LE][rows u16 LE]
//!   MSG_DATA   0x01  both     raw terminal bytes
//!   MSG_RESIZE 0x02  host→vm  [cols u16 LE][rows u16 LE]

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::mpsc;

const VMADDR_CID_ANY: u32 = u32::MAX;
const VSOCK_PORT: u32 = 5000;

const MSG_OPEN: u8 = 0x00;
const MSG_DATA: u8 = 0x01;
const MSG_RESIZE: u8 = 0x02;

// ── vsock primitives ────────────────────────────────────────────────────────

#[repr(C)]
struct SockAddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_flags: u8,
    svm_zero: [u8; 3],
}

fn vsock_listener(port: u32) -> io::Result<AsyncFd<OwnedFd>> {
    unsafe {
        let fd = libc::socket(
            libc::AF_VSOCK,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let addr = SockAddrVm {
            svm_family: libc::AF_VSOCK as u16,
            svm_reserved1: 0,
            svm_port: port,
            svm_cid: VMADDR_CID_ANY,
            svm_flags: 0,
            svm_zero: [0; 3],
        };
        let addrlen = std::mem::size_of::<SockAddrVm>() as libc::socklen_t;
        if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, addrlen) < 0 {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }
        if libc::listen(fd, 8) < 0 {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }
        AsyncFd::new(OwnedFd::from_raw_fd(fd)).map_err(io::Error::from)
    }
}

async fn vsock_accept(listener: &AsyncFd<OwnedFd>) -> io::Result<OwnedFd> {
    loop {
        let mut guard = listener.readable().await?;
        match guard.try_io(|inner| {
            let fd = unsafe {
                libc::accept4(
                    inner.get_ref().as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                )
            };
            if fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(unsafe { OwnedFd::from_raw_fd(fd) })
            }
        }) {
            Ok(r) => return r,
            Err(_would_block) => continue,
        }
    }
}

// ── VsockStream: AsyncRead + AsyncWrite over a raw fd ───────────────────────

struct VsockStream {
    fd: AsyncFd<OwnedFd>,
}

impl VsockStream {
    fn new(fd: OwnedFd) -> io::Result<Self> {
        Ok(Self { fd: AsyncFd::new(fd)? })
    }
}

impl AsyncRead for VsockStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.fd.poll_read_ready(cx))?;
            let result = guard.try_io(|inner| {
                let unfilled = buf.initialize_unfilled();
                let n = unsafe {
                    libc::read(
                        inner.get_ref().as_raw_fd(),
                        unfilled.as_mut_ptr() as *mut libc::c_void,
                        unfilled.len(),
                    )
                };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    buf.advance(n as usize);
                    Ok(())
                }
            });
            match result {
                Ok(r) => return Poll::Ready(r),
                Err(_would_block) => continue,
            }
        }
    }
}

impl AsyncWrite for VsockStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.fd.poll_write_ready(cx))?;
            let result = guard.try_io(|inner| {
                let n = unsafe {
                    libc::write(
                        inner.get_ref().as_raw_fd(),
                        buf.as_ptr() as *const libc::c_void,
                        buf.len(),
                    )
                };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            });
            match result {
                Ok(r) => return Poll::Ready(r),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

// ── Frame protocol helpers ──────────────────────────────────────────────────

async fn read_frame(r: &mut (impl AsyncRead + Unpin)) -> io::Result<(u8, Vec<u8>)> {
    let msg_type = r.read_u8().await?;
    let len = r.read_u32_le().await?;
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok((msg_type, buf))
}

async fn write_frame(
    w: &mut (impl AsyncWrite + Unpin),
    msg_type: u8,
    payload: &[u8],
) -> io::Result<()> {
    w.write_u8(msg_type).await?;
    w.write_u32_le(payload.len() as u32).await?;
    w.write_all(payload).await?;
    Ok(())
}

// ── Session handler ─────────────────────────────────────────────────────────

async fn handle(conn_fd: OwnedFd) -> anyhow::Result<()> {
    let stream = VsockStream::new(conn_fd)?;
    let (mut rx, mut tx) = tokio::io::split(stream);

    // First frame must be MSG_OPEN carrying the initial terminal size.
    let (msg_type, payload) = read_frame(&mut rx).await?;
    anyhow::ensure!(msg_type == MSG_OPEN && payload.len() == 4, "expected MSG_OPEN");
    let cols = u16::from_le_bytes([payload[0], payload[1]]);
    let rows = u16::from_le_bytes([payload[2], payload[3]]);

    // Open PTY and spawn a login shell.
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
    let mut cmd = CommandBuilder::new("/bin/bash");
    cmd.env("TERM", "xterm-256color");
    let _child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    // Detach sync reader/writer from the PTY master.
    let mut pty_reader = pair.master.try_clone_reader()?;
    let mut pty_writer = pair.master.take_writer()?;

    // Blocking thread: PTY master → mpsc channel.
    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(32);
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if pty_out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Blocking thread: mpsc channel → PTY master (input).
    let (pty_in_tx, mut pty_in_rx) = mpsc::channel::<Vec<u8>>(32);
    std::thread::spawn(move || {
        while let Some(data) = pty_in_rx.blocking_recv() {
            if pty_writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    // Async relay loop.
    loop {
        tokio::select! {
            data = pty_out_rx.recv() => {
                match data {
                    None => break,
                    Some(d) => write_frame(&mut tx, MSG_DATA, &d).await?,
                }
            }
            frame = read_frame(&mut rx) => {
                match frame? {
                    (MSG_DATA, d) => {
                        if pty_in_tx.send(d).await.is_err() { break; }
                    }
                    (MSG_RESIZE, d) if d.len() == 4 => {
                        let cols = u16::from_le_bytes([d[0], d[1]]);
                        let rows = u16::from_le_bytes([d[2], d[3]]);
                        let _ = pair.master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

// ── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let listener = vsock_listener(VSOCK_PORT).expect("failed to listen on vsock");
    eprintln!("vm-shell listening on vsock port {VSOCK_PORT}");
    loop {
        match vsock_accept(&listener).await {
            Ok(fd) => {
                tokio::spawn(async move {
                    if let Err(e) = handle(fd).await {
                        eprintln!("session error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}
