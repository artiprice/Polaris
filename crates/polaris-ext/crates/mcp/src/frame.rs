use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, ReadBuf};

/// Maximum bytes accepted in one newline-delimited JSON-RPC frame.
pub const MAX_JSONRPC_FRAME_BYTES: usize = 786_432;
const MAX_READ_CHUNK_BYTES: usize = 16_384;

/// Enforces a per-frame bound before rmcp's stream parser can grow its line buffer.
pub struct BoundedFrameReader<R> {
    inner: R,
    max_frame_bytes: usize,
    current_frame_bytes: usize,
}

impl<R> BoundedFrameReader<R> {
    /// Creates a reader that rejects oversized newline-delimited frames.
    /// Creates a reader that rejects frames above the protocol bound.
    ///
    /// # Errors
    /// Returns `InvalidInput` for a zero or oversized frame limit.
    pub fn new(inner: R, max_frame_bytes: usize) -> io::Result<Self> {
        if max_frame_bytes == 0 || max_frame_bytes > MAX_JSONRPC_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid MCP frame limit",
            ));
        }
        Ok(Self {
            inner,
            max_frame_bytes,
            current_frame_bytes: 0,
        })
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedFrameReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let read_capacity = output.remaining().min(MAX_READ_CHUNK_BYTES);
        let mut chunk = vec![0_u8; read_capacity];
        let mut chunk_output = ReadBuf::new(&mut chunk);
        match Pin::new(&mut this.inner).poll_read(context, &mut chunk_output) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {
                let bytes = chunk_output.filled();
                let mut frame_bytes = this.current_frame_bytes;
                for byte in bytes {
                    if *byte == b'\n' {
                        frame_bytes = 0;
                    } else {
                        frame_bytes = match frame_bytes.checked_add(1) {
                            Some(next_size) => next_size,
                            None => {
                                return Poll::Ready(Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "MCP frame size overflow",
                                )));
                            }
                        };
                        if frame_bytes > this.max_frame_bytes {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "MCP frame exceeds configured limit",
                            )));
                        }
                    }
                }
                this.current_frame_bytes = frame_bytes;
                output.put_slice(bytes);
                Poll::Ready(Ok(()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::BoundedFrameReader;

    #[test]
    fn accepts_multiple_bounded_frames() {
        let runtime = tokio::runtime::Builder::new_current_thread().build();
        assert!(runtime.is_ok());
        if let Ok(runtime) = runtime {
            runtime.block_on(async {
                let (mut writer, reader) = tokio::io::duplex(64);
                let write_result = writer.write_all(b"{}\n[]\n").await;
                assert!(write_result.is_ok());
                drop(writer);
                let bounded = BoundedFrameReader::new(reader, 4);
                assert!(bounded.is_ok());
                if let Ok(mut bounded) = bounded {
                    let mut result = Vec::new();
                    let read_result = bounded.read_to_end(&mut result).await;
                    assert!(read_result.is_ok());
                    assert_eq!(result, b"{}\n[]\n");
                }
            });
        }
    }

    #[test]
    fn rejects_oversized_frame_before_returning_its_bytes() {
        let runtime = tokio::runtime::Builder::new_current_thread().build();
        assert!(runtime.is_ok());
        if let Ok(runtime) = runtime {
            runtime.block_on(async {
                let (mut writer, reader) = tokio::io::duplex(64);
                let write_result = writer.write_all(b"12345\n").await;
                assert!(write_result.is_ok());
                drop(writer);
                let bounded = BoundedFrameReader::new(reader, 4);
                assert!(bounded.is_ok());
                if let Ok(mut bounded) = bounded {
                    let mut result = Vec::new();
                    let read_result = bounded.read_to_end(&mut result).await;
                    assert!(read_result.is_err());
                    assert!(result.is_empty());
                }
            });
        }
    }
}
