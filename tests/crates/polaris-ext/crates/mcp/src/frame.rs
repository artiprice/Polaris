// Unit tests of crates/polaris-ext/crates/mcp/src/frame.rs (`tests`); compiled into that module via #[path].
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
