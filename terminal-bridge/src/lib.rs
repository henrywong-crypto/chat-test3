use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::openpty;
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use thiserror::Error;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Nix(#[from] nix::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

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

pub fn resize_pty_fd(fd: RawFd, rows: u16, cols: u16) -> Result<()> {
    let winsize = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    let ret = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) };
    if ret == -1 {
        return Err(Error::Io(io::Error::last_os_error()));
    }
    Ok(())
}

pub fn resize_pty(pty_master: &PtyMaster, terminal_size: &TerminalSize) -> Result<()> {
    resize_pty_fd(pty_master.as_raw_fd(), terminal_size.rows, terminal_size.cols)
}

fn set_raw_mode(fd: &OwnedFd) -> Result<()> {
    let mut termios = tcgetattr(fd)?;
    cfmakeraw(&mut termios);
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
                    libc::read(fd, unfilled.as_mut_ptr() as *mut libc::c_void, unfilled.len())
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
                    libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len())
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
