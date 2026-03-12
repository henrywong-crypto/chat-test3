use bytes::Bytes;
use futures::{channel::mpsc, SinkExt};
use std::io;

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

    pub fn flush_remaining(&mut self) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt, StreamExt};
    use std::io::{Seek, SeekFrom, Write};

    fn make_writer() -> (
        SeekableChannelWriter,
        mpsc::Receiver<Result<Bytes, io::Error>>,
    ) {
        let (tx, rx) = mpsc::channel(64);
        (SeekableChannelWriter::new(tx), rx)
    }

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
    fn test_write_fills_buffer_without_flushing() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.buf, [1, 2, 3, 4]);
        assert_eq!(writer.pos, 4);
        assert_eq!(writer.high_water, 4);
    }

    #[test]
    fn test_sequential_writes_build_buffer() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3]).unwrap();
        writer.write_all(&[4, 5, 6]).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.buf, [1, 2, 3, 4, 5, 6]);
        assert_eq!(writer.pos, 6);
        assert_eq!(writer.high_water, 6);
    }

    #[test]
    fn test_write_after_seek_back_overwrites_without_extending_high_water() {
        let (mut writer, mut rx) = make_writer();
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
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[0u8; 8]).unwrap();
        let pos = writer.seek(SeekFrom::Start(3)).unwrap();
        assert_eq!(pos, 3);
        assert_eq!(writer.pos, 3);
        assert!(drain_channel(&mut rx).is_empty()); // seek-back doesn't flush
    }

    #[test]
    fn test_seek_from_current_positive() {
        let (mut writer, _rx) = make_writer();
        writer.write_all(&[0u8; 8]).unwrap();
        writer.seek(SeekFrom::Start(2)).unwrap();
        let pos = writer.seek(SeekFrom::Current(3)).unwrap();
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_seek_from_current_negative() {
        let (mut writer, _rx) = make_writer();
        writer.write_all(&[0u8; 8]).unwrap();
        let pos = writer.seek(SeekFrom::Current(-3)).unwrap();
        assert_eq!(pos, 5);
    }

    #[test]
    fn test_seek_from_end_returns_unsupported() {
        let (mut writer, _rx) = make_writer();
        let err = writer.seek(SeekFrom::End(0)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn test_seek_before_base_returns_invalid_input() {
        let (mut writer, mut rx) = make_writer();
        // Write 4 bytes then seek to high_water to advance base to 4
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        writer.seek(SeekFrom::Start(4)).unwrap();
        drain_channel(&mut rx);
        // base is now 4 — seeking before it must fail
        let err = writer.seek(SeekFrom::Start(2)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_seek_back_below_high_water_does_not_flush() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        writer.seek(SeekFrom::Start(0)).unwrap(); // new_pos < high_water
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.base, 0); // base unchanged
    }

    #[test]
    fn test_seek_to_high_water_triggers_flush() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        writer.seek(SeekFrom::Start(4)).unwrap(); // exactly at high_water
        let flushed = drain_channel(&mut rx);
        assert_eq!(flushed, [1, 2, 3, 4]);
        assert_eq!(writer.buf.len(), 0);
        assert_eq!(writer.base, 4);
    }

    #[test]
    fn test_seek_beyond_high_water_triggers_flush() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        writer.seek(SeekFrom::Start(8)).unwrap(); // beyond high_water
        let flushed = drain_channel(&mut rx);
        assert_eq!(flushed, [1, 2, 3, 4]);
        assert_eq!(writer.base, 4);
    }

    #[test]
    fn test_seek_at_zero_on_fresh_writer_is_noop() {
        // high_water == base == 0 → flush_len == 0 → no-op
        let (mut writer, mut rx) = make_writer();
        writer.seek(SeekFrom::Start(0)).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
    }

    // ── flush_remaining ───────────────────────────────────────────────────────

    #[test]
    fn test_flush_remaining_on_empty_buf_is_noop() {
        let (mut writer, mut rx) = make_writer();
        writer.flush_remaining().unwrap();
        assert!(drain_channel(&mut rx).is_empty());
    }

    #[test]
    fn test_flush_remaining_sends_unflushed_bytes() {
        let (mut writer, mut rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        writer.flush_remaining().unwrap();
        assert_eq!(drain_channel(&mut rx), [1, 2, 3, 4]);
    }

    #[test]
    fn test_flush_remaining_receiver_dropped_returns_broken_pipe() {
        let (mut writer, rx) = make_writer();
        drop(rx);
        writer.write_all(&[1, 2, 3]).unwrap();
        let err = writer.flush_remaining().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn test_seek_flush_receiver_dropped_returns_broken_pipe() {
        let (mut writer, rx) = make_writer();
        writer.write_all(&[1, 2, 3, 4]).unwrap();
        drop(rx);
        let err = writer.seek(SeekFrom::Start(4)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    // ── full zip pattern ──────────────────────────────────────────────────────

    #[test]
    fn test_full_zip_pattern() {
        // Simulates what zip does for a single file entry:
        //   1. Write local header with placeholder CRC/sizes
        //   2. Write compressed data
        //   3. Seek BACK to header start
        //   4. Overwrite header with real CRC/sizes
        //   5. Seek FORWARD to end of data → flush
        //   6. Write central directory → flush_remaining
        let (mut writer, mut rx) = make_writer();

        let header_placeholder = [0x00u8; 8];
        let data = [0xAB, 0xCD, 0xEF, 0x01, 0x02, 0x03, 0x04, 0x05];
        let header_final = [0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0xF8];
        let central_dir = [0x50, 0x4B, 0x05, 0x06];

        // Step 1 — placeholder header
        writer.write_all(&header_placeholder).unwrap();
        assert!(drain_channel(&mut rx).is_empty());

        // Step 2 — compressed data
        writer.write_all(&data).unwrap();
        assert!(drain_channel(&mut rx).is_empty());
        assert_eq!(writer.high_water, 16);

        // Step 3 — seek back to header start
        writer.seek(SeekFrom::Start(0)).unwrap();
        assert!(drain_channel(&mut rx).is_empty()); // no flush on seek-back

        // Step 4 — overwrite with real header
        writer.write_all(&header_final).unwrap();
        assert!(drain_channel(&mut rx).is_empty()); // still no flush
        assert_eq!(writer.high_water, 16); // high_water unchanged

        // Step 5 — seek forward to end → flush
        writer.seek(SeekFrom::Start(16)).unwrap();
        let flushed = drain_channel(&mut rx);
        let mut expected = Vec::new();
        expected.extend_from_slice(&header_final);
        expected.extend_from_slice(&data);
        assert_eq!(flushed, expected);
        assert_eq!(writer.buf.len(), 0);
        assert_eq!(writer.base, 16);

        // Step 6 — central directory (no seek-back; flushed via flush_remaining)
        writer.write_all(&central_dir).unwrap();
        writer.flush_remaining().unwrap();
        assert_eq!(drain_channel(&mut rx), central_dir);
    }
}
