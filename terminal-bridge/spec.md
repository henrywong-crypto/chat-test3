# terminal-bridge

- PTY master is a bidirectional byte stream (AsyncRead + AsyncWrite)
- PTY slave fd is passed to the firecracker process at spawn time
- Wire protocol message types: `input`, `output`, `resize`

## Functions

```
open_pty() -> Result<PtyPair>
resize_pty(pty_master: &PtyMaster, terminal_size: &TerminalSize) -> Result<()>
```
