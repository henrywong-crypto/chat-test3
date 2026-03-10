# pty

Async PTY master/slave pair with raw mode and non-blocking I/O for tokio.

## Responsibilities

- Open a PTY pair via `openpty`
- Set the master to raw mode with `OPOST|ONLCR` re-enabled (so `\n` becomes `\r\n` for xterm.js)
- Set the master to non-blocking mode for use with `AsyncFd`
- Expose `PtyMaster` as `AsyncRead + AsyncWrite` for use in tokio tasks
- Support PTY window resize via `TIOCSWINSZ`

## API

```
open_pty() -> Result<PtyPair>
resize_pty(pty_master: &PtyMaster, terminal_size: &TerminalSize) -> Result<()>
resize_pty_fd(fd: RawFd, rows: u16, cols: u16) -> Result<()>
```

## Types

- `PtyPair { master: PtyMaster, slave: PtySlave }`
- `PtyMaster` — implements `AsyncRead + AsyncWrite + AsRawFd`
- `PtySlave` — wraps `OwnedFd`; call `into_owned_fd()` to pass to a child process
- `TerminalSize { rows: u16, cols: u16 }`
