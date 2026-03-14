use bytes::Bytes;
use futures::{SinkExt, channel::mpsc};
use std::io;
use tokio::{
    runtime::Handle,
    time::{Duration, timeout},
};

const SEND_TIMEOUT_SECS: u64 = 300;

// zip 2.x requires Write + Seek. For each file it follows this pattern:
//   1. Write local header (pos → header_end)
//   2. Write compressed data (pos → file_end = high_water)
//   3. Seek BACK to header_start to overwrite with real CRC/sizes
//   4. Write updated header (pos → header_end again)
//   5. Seek FORWARD to file_end (pos == high_water) → flush all buffered bytes
//
// At step 5 every byte for that entry is finalised and can be sent to the channel.
// The buffer drains to zero and the cycle repeats, so memory stays O(largest file).

pub struct SeekableChannelWriter {
    buf: Vec<u8>,    // unflushed bytes; buf[0] is at logical offset `base`
    base: u64,       // logical stream offset of buf[0]
    pos: u64,        // current read/write position
    high_water: u64, // highest position ever reached
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
}

impl SeekableChannelWriter {
    pub fn new(tx: mpsc::Sender<Result<Bytes, io::Error>>) -> Self {
        Self {
            buf: Vec::new(),
            base: 0,
            pos: 0,
            high_water: 0,
            tx,
        }
    }

    fn flush_to_high_water(&mut self) -> io::Result<()> {
        let flush_len: usize = self
            .high_water
            .checked_sub(self.base)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "high_water is behind base"))?
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "flush offset overflows usize"))?;
        if flush_len == 0 {
            return Ok(());
        }
        if flush_len > self.buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "flush_len exceeds buffer",
            ));
        }
        let chunk = Bytes::copy_from_slice(&self.buf[..flush_len]);
        match Handle::current().block_on(timeout(
            Duration::from_secs(SEND_TIMEOUT_SECS),
            self.tx.send(Ok(chunk)),
        )) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "zip receiver dropped",
                ))
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "zip send timed out",
                ))
            }
        }
        self.buf.drain(..flush_len);
        self.base = self.high_water;
        Ok(())
    }

    pub fn flush_remaining(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::copy_from_slice(&self.buf);
        match Handle::current().block_on(timeout(
            Duration::from_secs(SEND_TIMEOUT_SECS),
            self.tx.send(Ok(chunk)),
        )) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "zip receiver dropped",
                ))
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "zip send timed out",
                ))
            }
        }
        Ok(())
    }
}

impl io::Write for SeekableChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let start: usize = self
            .pos
            .checked_sub(self.base)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "pos is behind base"))?
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "write offset overflows usize"))?;
        let end = start
            .checked_add(data.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "write end overflows usize"))?;
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.buf[start..end].copy_from_slice(data);
        self.pos = self
            .base
            .checked_add(
                u64::try_from(end)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "write end overflows u64"))?,
            )
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "write position overflows u64"))?;
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
            io::SeekFrom::Current(off) => self
                .pos
                .checked_add_signed(off)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek position overflow"))?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt, StreamExt};
    use std::io::{Seek, SeekFrom, Write};

    // Drain all items currently available in the channel without blocking.
    fn drain_channel(rx: &mut mpsc::Receiver<Result<Bytes, io::Error>>) -> Vec<u8> {
        let mut bytes = Vec::new();
        while let Some(Some(Ok(chunk))) = rx.next().now_or_never() {
            bytes.extend_from_slice(&chunk);
        }
        bytes
    }

    // ── write ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_write_position_overflow_returns_invalid_input() {
        let (tx, _rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.base = u64::MAX;
        writer.pos = u64::MAX;
        writer.high_water = u64::MAX;
        let err = writer.write(&[1]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_write_fills_buffer_without_flushing() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.buf, [1, 2, 3, 4]);
        assert_eq!(writer.pos, 4);
        assert_eq!(writer.high_water, 4);
    }

    #[test]
    fn test_sequential_writes_build_buffer() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[1, 2, 3]).unwrap();
        writer.write_all(&[4, 5, 6]).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.buf, [1, 2, 3, 4, 5, 6]);
        assert_eq!(writer.pos, 6);
        assert_eq!(writer.high_water, 6);
    }

    #[test]
    fn test_write_after_seek_back_overwrites_without_extending_high_water() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer
            .write_all(&[0x00, 0x00, 0x00, 0x00, 5, 6, 7, 8])
            .unwrap();
        writer.seek(SeekFrom::Start(0)).unwrap();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.high_water, 8); // high_water unchanged
        assert_eq!(writer.pos, 4);
        assert_eq!(&writer.buf[..4], &[1, 2, 3, 4]); // first 4 bytes overwritten
        assert_eq!(&writer.buf[4..], &[5, 6, 7, 8]); // rest untouched
    }

    // ── seek ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_seek_from_start_moves_pos() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[0u8; 8]).unwrap();
        let pos = writer.seek(SeekFrom::Start(3)).unwrap();
        assert_eq!(pos, 3);
        assert_eq!(writer.pos, 3);
        assert!(drain_channel(&mut rx).is_empty()); // seek-back doesn't flush
    }

    #[test]
    fn test_seek_from_current_positive() {
        let (tx, _rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[0u8; 8]).unwrap();
        writer.seek(SeekFrom::Start(2)).unwrap();
        let pos = writer.seek(SeekFrom::Current(3)).unwrap();
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_seek_from_current_negative() {
        let (tx, _rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[0u8; 8]).unwrap();
        let pos = writer.seek(SeekFrom::Current(-3)).unwrap();
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_seek_from_end_returns_unsupported() {
        let (tx, _rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        let err = writer.seek(SeekFrom::End(0)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn test_seek_back_below_high_water_does_not_flush() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.write_all(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        writer.seek(SeekFrom::Start(0)).unwrap(); // new_pos < high_water
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.base, 0); // base unchanged
    }

    #[test]
    fn test_seek_at_zero_on_fresh_writer_is_noop() {
        // high_water == base == 0 → flush_len == 0 → no-op
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.seek(SeekFrom::Start(0)).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn test_seek_before_base_returns_invalid_input() {
        let (tx, mut rx) = mpsc::channel(64);
        let err = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3, 4]).unwrap();
            writer.seek(SeekFrom::Start(4)).unwrap(); // flush, base → 4
            writer.seek(SeekFrom::Start(2)).unwrap_err() // seek before base → error
        })
        .await
        .unwrap();
        drain_channel(&mut rx);
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn test_seek_to_high_water_triggers_flush() {
        let (tx, mut rx) = mpsc::channel(64);
        let (buf_len, base) = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3, 4]).unwrap();
            writer.seek(SeekFrom::Start(4)).unwrap(); // exactly at high_water
            (writer.buf.len(), writer.base)
        })
        .await
        .unwrap();
        let flushed = drain_channel(&mut rx);
        assert_eq!(flushed, [1, 2, 3, 4]);
        assert_eq!(buf_len, 0);
        assert_eq!(base, 4);
    }

    #[tokio::test]
    async fn test_seek_beyond_high_water_triggers_flush() {
        let (tx, mut rx) = mpsc::channel(64);
        let base = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3, 4]).unwrap();
            writer.seek(SeekFrom::Start(8)).unwrap(); // beyond high_water
            writer.base
        })
        .await
        .unwrap();
        let flushed = drain_channel(&mut rx);
        assert_eq!(flushed, [1, 2, 3, 4]);
        assert_eq!(base, 4);
    }

    // ── flush_remaining ───────────────────────────────────────────────────────

    #[test]
    fn test_flush_remaining_on_empty_buf_is_noop() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut writer = SeekableChannelWriter::new(tx);
        writer.flush_remaining().unwrap();
        assert!(drain_channel(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn test_flush_remaining_sends_unflushed_bytes() {
        let (tx, mut rx) = mpsc::channel(64);
        tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3, 4]).unwrap();
            writer.flush_remaining().unwrap();
        })
        .await
        .unwrap();
        assert_eq!(drain_channel(&mut rx), [1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn test_flush_remaining_receiver_dropped_returns_broken_pipe() {
        let (tx, rx) = mpsc::channel(64);
        drop(rx);
        let err = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3]).unwrap();
            writer.flush_remaining().unwrap_err()
        })
        .await
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn test_seek_flush_receiver_dropped_returns_broken_pipe() {
        let (tx, rx) = mpsc::channel(64);
        drop(rx);
        let err = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);
            writer.write_all(&[1, 2, 3, 4]).unwrap();
            writer.seek(SeekFrom::Start(4)).unwrap_err()
        })
        .await
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    // ── full zip pattern ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_full_zip_pattern() {
        // Simulates what zip does for a single file entry:
        //   1. Write local header with placeholder CRC/sizes
        //   2. Write compressed data
        //   3. Seek BACK to header start
        //   4. Overwrite header with real CRC/sizes
        //   5. Seek FORWARD to end of data → flush
        //   6. Write central directory → flush_remaining
        let (tx, mut rx) = mpsc::channel(64);

        let (first_flushed_expected, central_dir) = tokio::task::spawn_blocking(move || {
            let mut writer = SeekableChannelWriter::new(tx);

            let header_placeholder = [0x00u8; 8];
            let data = [0xAB, 0xCD, 0xEF, 0x01, 0x02, 0x03, 0x04, 0x05];
            let header_final = [0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0xF8];
            let central_dir = [0x50, 0x4B, 0x05, 0x06];

            // Step 1 — placeholder header
            writer.write_all(&header_placeholder).unwrap();

            // Step 2 — compressed data
            writer.write_all(&data).unwrap();
            assert_eq!(writer.high_water, 16);

            // Step 3 — seek back to header start
            writer.seek(SeekFrom::Start(0)).unwrap();

            // Step 4 — overwrite with real header
            writer.write_all(&header_final).unwrap();
            assert_eq!(writer.high_water, 16); // high_water unchanged

            // Step 5 — seek forward to end → flush
            writer.seek(SeekFrom::Start(16)).unwrap();

            // Step 6 — central directory (no seek-back; flushed via flush_remaining)
            writer.write_all(&central_dir).unwrap();
            writer.flush_remaining().unwrap();

            let mut expected = Vec::new();
            expected.extend_from_slice(&header_final);
            expected.extend_from_slice(&data);
            (expected, central_dir)
        })
        .await
        .unwrap();

        let all_bytes = drain_channel(&mut rx);
        let (first_chunk, second_chunk) = all_bytes.split_at(16);
        assert_eq!(first_chunk, first_flushed_expected);
        assert_eq!(second_chunk, central_dir);
    }
}
