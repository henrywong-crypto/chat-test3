use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};
use anyhow::{Context as _, Result};
use libc::{c_void, read as libc_read, write as libc_write};
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    sys::termios::{cfmakeraw, tcgetattr, tcsetattr, OutputFlags, SetArg},
};
use rustix::termios::{tcsetwinsize, Winsize};
use tokio::io::{unix::AsyncFd, AsyncRead, AsyncWrite, ReadBuf};

pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

pub struct PtyPair {
    pub master: PtyMaster,
    pub slave: PtySlave,
}

pub struct PtyMaster {
    fd: AsyncFd<OwnedFd>,
}

impl AsRawFd for PtyMaster {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

pub struct PtySlave {
    fd: OwnedFd,
}

impl PtySlave {
    pub fn into_owned_fd(self) -> OwnedFd {
        self.fd
    }
}

pub fn open_pty() -> Result<PtyPair> {
    let pty = openpty(None, None)?;
    set_raw_mode(&pty.master)?;
    set_nonblocking_fd(&pty.master)?;
    let master = PtyMaster { fd: AsyncFd::new(pty.master)? };
    let slave = PtySlave { fd: pty.slave };
    Ok(PtyPair { master, slave })
}

pub fn resize_pty(pty_master: &PtyMaster, terminal_size: &TerminalSize) -> Result<()> {
    resize_pty_fd(pty_master.as_raw_fd(), terminal_size.rows, terminal_size.cols)
}

pub fn resize_pty_fd(fd: RawFd, rows: u16, cols: u16) -> Result<()> {
    let winsize = Winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
    tcsetwinsize(borrowed_fd, winsize)
        .map_err(io::Error::from)
        .context("failed to resize pty")
}

fn set_raw_mode(fd: &OwnedFd) -> Result<()> {
    let mut termios = tcgetattr(fd)?;
    cfmakeraw(&mut termios);
    // cfmakeraw clears OPOST which disables all output processing, making ONLCR
    // a no-op. Re-enable OPOST + ONLCR so \n from the VM becomes \r\n for xterm.js.
    termios.output_flags |= OutputFlags::OPOST | OutputFlags::ONLCR;
    tcsetattr(fd, SetArg::TCSANOW, &termios)?;
    Ok(())
}

fn set_nonblocking_fd(fd: &OwnedFd) -> Result<()> {
    let flags = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL)?;
    let flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
    fcntl(fd.as_raw_fd(), FcntlArg::F_SETFL(flags))?;
    Ok(())
}

impl AsyncRead for PtyMaster {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.fd.poll_read_ready(cx))?;
            let result = guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                let unfilled = buf.initialize_unfilled();
                let n = unsafe {
                    libc_read(fd, unfilled.as_mut_ptr() as *mut c_void, unfilled.len())
                };
                if n == -1 {
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

impl AsyncWrite for PtyMaster {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.fd.poll_write_ready(cx))?;
            let result = guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                let n = unsafe {
                    libc_write(fd, buf.as_ptr() as *const c_void, buf.len())
                };
                if n == -1 {
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
