//! Bounded, byte-preserving PTY sessions backed by `portable-pty`.

use std::io::{Read, Write};
use std::sync::{Mutex, mpsc};

use portable_pty::{CommandBuilder, MasterPty, PtySize as NativePtySize, native_pty_system};
use thiserror::Error;

use crate::EnvPolicy;

const OUTPUT_QUEUE_CAPACITY: usize = 32;
const READ_CHUNK_CAPACITY: usize = 8 * 1024;

/// Terminal dimensions requested from the operating-system PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    /// Visible rows; zero is rejected.
    pub rows: u16,
    /// Visible columns; zero is rejected.
    pub columns: u16,
    /// Cell width in pixels when supported.
    pub pixel_width: u16,
    /// Cell height in pixels when supported.
    pub pixel_height: u16,
}

impl From<PtySize> for NativePtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.columns,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

/// PTY setup, I/O, or child-process failure.
#[derive(Debug, Error)]
pub enum PtyError {
    /// Program path was not absolute.
    #[error("PTY program path must be absolute")]
    RelativeProgram,
    /// Terminal dimensions must be nonzero.
    #[error("PTY rows and columns must be nonzero")]
    InvalidSize,
    /// The PTY API returned an error.
    #[error("PTY operation failed: {0}")]
    Native(String),
    /// A synchronization primitive was poisoned.
    #[error("PTY lock was poisoned")]
    Poisoned,
    /// Child output or input failed.
    #[error("PTY I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// One bounded output record. Bytes are preserved exactly and may split UTF-8 code points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyOutput {
    /// Raw output bytes from the terminal.
    Data(Vec<u8>),
    /// Reader reached end of stream.
    Closed,
    /// Reader failed after session startup.
    Failed(String),
}

/// Live PTY child with bounded output and resize/input controls.
pub struct PtySession {
    master: Box<dyn MasterPty>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    output: mpsc::Receiver<PtyOutput>,
}

impl PtySession {
    /// Receives one output record, waiting at most `timeout`.
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<PtyOutput, mpsc::RecvTimeoutError> {
        self.output.recv_timeout(timeout)
    }

    /// Writes bytes to the PTY without text conversion.
    pub fn write(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut writer = self.writer.lock().map_err(|_| PtyError::Poisoned)?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Resizes the terminal and notifies the attached process.
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        validate_size(size)?;
        self.master
            .resize(size.into())
            .map_err(|error| PtyError::Native(error.to_string()))
    }

    /// Returns the process exit code if the child has exited.
    pub fn try_wait(&self) -> Result<Option<u32>, PtyError> {
        let mut child = self.child.lock().map_err(|_| PtyError::Poisoned)?;
        child
            .try_wait()
            .map(|status| status.map(|value| value.exit_code()))
            .map_err(|error| PtyError::Native(error.to_string()))
    }

    /// Kills the PTY child process.
    pub fn kill(&self) -> Result<(), PtyError> {
        self.child
            .lock()
            .map_err(|_| PtyError::Poisoned)?
            .kill()
            .map_err(|error| PtyError::Native(error.to_string()))
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PtySession").finish_non_exhaustive()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if let Err(error) = child.kill() {
                tracing::debug!(error = %error, "PTY child was already stopped or could not be killed during drop");
            }
        }
    }
}

/// Starts a PTY child with a bounded output reader and a complete explicit environment.
pub fn spawn_pty(
    program: &std::path::Path,
    args: &[std::ffi::OsString],
    cwd: Option<&std::path::Path>,
    env: &EnvPolicy,
    size: PtySize,
) -> Result<PtySession, PtyError> {
    if !program.is_absolute() {
        return Err(PtyError::RelativeProgram);
    }
    #[cfg(windows)]
    if matches!(
        program
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("bat" | "cmd")
    ) {
        return Err(PtyError::Native(
            "batch scripts require an explicit trusted command policy".to_owned(),
        ));
    }
    validate_size(size)?;
    let system = native_pty_system();
    let pair = system
        .openpty(size.into())
        .map_err(|error| PtyError::Native(error.to_string()))?;
    let mut command = CommandBuilder::new(program.as_os_str());
    command.env_clear();
    command.args(args);
    if let Some(directory) = cwd {
        command.cwd(directory.as_os_str());
    }
    for (key, value) in env.entries() {
        command.env(key, value);
    }
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| PtyError::Native(error.to_string()))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| PtyError::Native(error.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| PtyError::Native(error.to_string()))?;
    let (sender, receiver) = mpsc::sync_channel(OUTPUT_QUEUE_CAPACITY);
    let reader_thread = std::thread::Builder::new()
        .name("polaris-pty-reader".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; READ_CHUNK_CAPACITY];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _send_result = sender.send(PtyOutput::Closed);
                        break;
                    }
                    Ok(count) => {
                        let record = buffer.get(..count).map(<[u8]>::to_vec);
                        let Some(bytes) = record else { break };
                        if sender.send(PtyOutput::Data(bytes)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _send_result = sender.send(PtyOutput::Failed(error.to_string()));
                        break;
                    }
                }
            }
        });
    let reader_thread = match reader_thread {
        Ok(handle) => handle,
        Err(error) => {
            if let Err(kill_error) = child.kill() {
                tracing::error!(error = %kill_error, "failed to clean up PTY child after reader thread startup failed");
            }
            return Err(PtyError::Io(error));
        }
    };
    drop(reader_thread);
    let master = pair.master;
    Ok(PtySession {
        master,
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        output: receiver,
    })
}

fn validate_size(size: PtySize) -> Result<(), PtyError> {
    if size.rows == 0 || size.columns == 0 {
        return Err(PtyError::InvalidSize);
    }
    Ok(())
}
