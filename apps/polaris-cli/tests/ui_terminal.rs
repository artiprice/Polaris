//! Physical terminal smoke tests against the actual CLI binary through a bounded PTY.
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};
use std::{
    error::Error,
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

const MAX_CAPTURE: usize = 2 * 1024 * 1024;
type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

enum ReadMessage {
    Bytes(Vec<u8>),
    Closed,
    Failed(String),
}

struct PtySession {
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    receiver: Receiver<ReadMessage>,
    bytes: Vec<u8>,
    cursor_query_answered: bool,
}

impl PtySession {
    fn spawn(
        state: &std::path::Path,
        workspace: &std::path::Path,
        initial: Option<&str>,
    ) -> TestResult<Self> {
        let system = native_pty_system();
        let pair = system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_polaris"));
        command.arg("--state-dir");
        command.arg(state);
        command.arg("--workspace");
        command.arg(workspace);
        command.arg("--color");
        command.arg("never");
        if let Some(initial) = initial {
            command.arg(initial);
        }
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (sender, receiver) = mpsc::sync_channel(8);
        thread::Builder::new()
            .name("polaris-pty-reader".to_owned())
            .spawn(move || {
                let mut reader = reader;
                let mut buffer = [0_u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            let _sent = sender.send(ReadMessage::Closed);
                            break;
                        }
                        Ok(count) => {
                            let Some(chunk) = buffer.get(..count) else {
                                let _sent = sender.send(ReadMessage::Failed(
                                    "PTY returned a read length outside the buffer".to_owned(),
                                ));
                                break;
                            };
                            if sender.send(ReadMessage::Bytes(chunk.to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _sent = sender.send(ReadMessage::Failed(error.to_string()));
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            child,
            _master: pair.master,
            writer,
            receiver,
            bytes: Vec::new(),
            cursor_query_answered: false,
        })
    }

    fn send(&mut self, bytes: &[u8]) -> TestResult<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> TestResult<()> {
        for character in text.chars() {
            let mut bytes = [0_u8; 4];
            self.send(character.encode_utf8(&mut bytes).as_bytes())?;
            thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    }

    fn receive(&mut self, timeout: Duration) -> TestResult<()> {
        match self.receiver.recv_timeout(timeout) {
            Ok(ReadMessage::Bytes(bytes)) => self.append(&bytes)?,
            Ok(ReadMessage::Closed) => {}
            Ok(ReadMessage::Failed(message)) => {
                return Err(io::Error::other(format!("PTY reader failed: {message}")).into());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {}
        }
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> TestResult<()> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("PTY capture size overflow"))?;
        if next > MAX_CAPTURE {
            return Err(io::Error::other("PTY capture exceeded 2 MiB").into());
        }
        self.bytes.extend_from_slice(bytes);
        if !self.cursor_query_answered && contains_bytes(&self.bytes, b"\x1b[6n") {
            // ConPTY transports device-status queries but does not act as the host terminal.
            self.writer.write_all(b"\x1b[1;1R")?;
            self.writer.flush()?;
            self.cursor_query_answered = true;
        }
        Ok(())
    }

    fn wait_for_text(&mut self, needle: &str, timeout: Duration) -> TestResult<()> {
        let deadline = deadline_after(timeout)?;
        loop {
            if contains_visible_text(&self.bytes, needle) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "PTY did not render {needle:?}; captured {} bytes; first bytes: {:02x?}; visible output: {}",
                        self.bytes.len(),
                        self.bytes
                            .get(..self.bytes.len().min(128))
                            .map_or(self.bytes.as_slice(), |prefix| prefix),
                        terminal_screen_text(&self.bytes)
                    ),
                )
                .into());
            }
            self.receive(remaining.min(Duration::from_millis(40)))?;
            if let Some(status) = self.child.try_wait()? {
                self.drain_for(Duration::from_millis(100))?;
                return Err(io::Error::other(format!(
                    "CLI exited before rendering {needle:?} with {status}; visible output: {}",
                    terminal_screen_text(&self.bytes)
                ))
                .into());
            }
        }
    }

    fn drain_for(&mut self, duration: Duration) -> TestResult<()> {
        let deadline = deadline_after(duration)?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            self.receive(remaining.min(Duration::from_millis(30)))?;
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> TestResult<ExitStatus> {
        let deadline = deadline_after(timeout)?;
        loop {
            self.receive(Duration::from_millis(25))?;
            if let Some(status) = self.child.try_wait()? {
                self.drain_for(Duration::from_millis(100))?;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "CLI did not exit before the PTY deadline",
                )
                .into());
            }
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _killed = self.child.kill();
    }
}

fn terminal_screen_text(bytes: &[u8]) -> String {
    const WIDTH: usize = 80;
    const HEIGHT: usize = 24;
    let input = String::from_utf8_lossy(bytes);
    let characters = input.chars().collect::<Vec<_>>();
    let mut cells = vec![vec![' '; WIDTH]; HEIGHT];
    let (mut x, mut y) = (0_usize, 0_usize);
    let mut index = 0_usize;

    while index < characters.len() {
        let Some(character) = characters.get(index).copied() else {
            break;
        };
        index = index.saturating_add(1);
        if character == '\x1b' {
            let Some(kind) = characters.get(index).copied() else {
                break;
            };
            index = index.saturating_add(1);
            if kind == '[' {
                let start = index;
                while let Some(byte) = characters.get(index) {
                    if ('\x40'..='\x7e').contains(byte) {
                        break;
                    }
                    index = index.saturating_add(1);
                }
                let Some(sequence) = characters.get(start..index) else {
                    break;
                };
                let parameters = sequence.iter().collect::<String>();
                let final_byte = characters.get(index).copied();
                index = index.saturating_add(1);
                let values = parameters
                    .trim_start_matches(['?', '>', '!'])
                    .split(';')
                    .filter_map(|value| value.parse::<usize>().ok())
                    .collect::<Vec<_>>();
                let first = values
                    .first()
                    .copied()
                    .filter(|value| *value != 0)
                    .map_or(1, |value| value);
                let second = values
                    .get(1)
                    .copied()
                    .filter(|value| *value != 0)
                    .map_or(1, |value| value);
                match final_byte {
                    Some('H' | 'f') => {
                        y = first.saturating_sub(1).min(HEIGHT.saturating_sub(1));
                        x = second.saturating_sub(1).min(WIDTH.saturating_sub(1));
                    }
                    Some('G') => x = first.saturating_sub(1).min(WIDTH.saturating_sub(1)),
                    Some('d') => y = first.saturating_sub(1).min(HEIGHT.saturating_sub(1)),
                    Some('A') => y = y.saturating_sub(first),
                    Some('B') => y = y.saturating_add(first).min(HEIGHT.saturating_sub(1)),
                    Some('C') => x = x.saturating_add(first).min(WIDTH.saturating_sub(1)),
                    Some('D') => x = x.saturating_sub(first),
                    Some('K') => {
                        let mode = values.first().copied().map_or(0, |value| value);
                        let row = cells.get_mut(y);
                        if let Some(row) = row {
                            let last = x.min(row.len().saturating_sub(1));
                            match mode {
                                1 => {
                                    if let Some(prefix) = row.get_mut(..=last) {
                                        prefix.fill(' ');
                                    }
                                }
                                2 => row.fill(' '),
                                _ => {
                                    if let Some(suffix) = row.get_mut(last..) {
                                        suffix.fill(' ');
                                    }
                                }
                            }
                        }
                    }
                    Some('J') if values.first().copied() == Some(2) => {
                        for row in &mut cells {
                            row.fill(' ');
                        }
                    }
                    _ => {}
                }
            } else if kind == ']' {
                while index < characters.len() {
                    let Some(character) = characters.get(index).copied() else {
                        break;
                    };
                    if character == '\x07' {
                        index = index.saturating_add(1);
                        break;
                    }
                    if character == '\x1b' && characters.get(index.saturating_add(1)) == Some(&'\\')
                    {
                        index = index.saturating_add(2);
                        break;
                    }
                    index = index.saturating_add(1);
                }
            }
            continue;
        }
        match character {
            '\r' => x = 0,
            '\n' => y = y.saturating_add(1).min(HEIGHT.saturating_sub(1)),
            '\t' => x = x.saturating_add(4).min(WIDTH.saturating_sub(1)),
            character if !character.is_control() => {
                if let Some(cell) = cells.get_mut(y).and_then(|row| row.get_mut(x)) {
                    *cell = character;
                }
                x = x.saturating_add(1).min(WIDTH.saturating_sub(1));
            }
            _ => {}
        }
    }

    cells
        .iter()
        .map(|row| row.iter().collect::<String>().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn contains_visible_text(bytes: &[u8], needle: &str) -> bool {
    let actual = terminal_screen_text(bytes)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let expected = needle
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    actual.contains(&expected)
}

fn deadline_after(duration: Duration) -> TestResult<Instant> {
    Instant::now()
        .checked_add(duration)
        .ok_or_else(|| io::Error::other("PTY deadline exceeded the monotonic clock range").into())
}

fn require(condition: bool, message: &str) -> TestResult<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message.to_owned()).into())
    }
}

fn new_dirs() -> TestResult<(tempfile::TempDir, tempfile::TempDir)> {
    Ok((tempfile::tempdir()?, tempfile::tempdir()?))
}

fn exit_with_quit(session: &mut PtySession) -> TestResult<()> {
    // The first Enter may accept the matching completion; the second submits it.
    session.type_text("/quit")?;
    session.send(b"\r\r")?;
    let status = session.wait_for_exit(Duration::from_secs(8))?;
    if !status.success() {
        return Err(io::Error::other(format!("CLI exited with {status}")).into());
    }
    Ok(())
}

#[test]
fn default_terminal_launch_shows_real_welcome_at_80x24() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    require(
        pty.bytes.len() <= MAX_CAPTURE,
        "PTY capture exceeded its bound",
    )?;
    require(
        !contains_visible_text(&pty.bytes, "SYNTHETIC"),
        "default welcome included synthetic fixture data",
    )?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
fn initial_prompt_is_visible_but_unsent_until_enter() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), Some("$websearch(on)"))?;
    pty.wait_for_text("$websearch(on)", Duration::from_secs(10))?;
    pty.drain_for(Duration::from_millis(120))?;
    require(
        !contains_visible_text(&pty.bytes, "Directive committed by the kernel"),
        "initial prompt was committed without Enter",
    )?;
    pty.send(b"\x03")?;
    pty.drain_for(Duration::from_millis(100))?;
    exit_with_quit(&mut pty)?;
    require(
        !contains_visible_text(&pty.bytes, "Directive committed by the kernel"),
        "cleared initial prompt unexpectedly committed",
    )?;
    Ok(())
}

#[test]
fn bracketed_paste_is_literal_and_never_submitted() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    pty.send(b"\x1b[200~$websearch(on)\x1b[201~")?;
    pty.wait_for_text("$websearch(on)", Duration::from_secs(5))?;
    pty.drain_for(Duration::from_millis(150))?;
    require(
        !contains_visible_text(&pty.bytes, "Directive committed by the kernel"),
        "bracketed paste submitted its contents",
    )?;
    pty.send(b"\x03")?;
    pty.drain_for(Duration::from_millis(100))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
fn valid_directive_commits_and_renders_receipt() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    pty.type_text("$websearch(on)")?;
    pty.wait_for_text("$websearch(on)", Duration::from_secs(5))?;
    pty.send(b"\r\r")?;
    pty.wait_for_text("Directive committed by the kernel", Duration::from_secs(8))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
fn quit_restores_terminal_modes_and_cursor() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    exit_with_quit(&mut pty)?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[?1049h"),
        "alternate screen was not entered",
    )?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[?1049l"),
        "alternate screen was not restored",
    )?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[?2004h"),
        "bracketed paste was not enabled",
    )?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[?2004l"),
        "bracketed paste was not disabled",
    )?;
    #[cfg(not(windows))]
    {
        require(
            contains_bytes(&pty.bytes, b"\x1b[?1000h"),
            "mouse capture was not enabled",
        )?;
        require(
            contains_bytes(&pty.bytes, b"\x1b[?1000l"),
            "mouse capture was not disabled",
        )?;
    }
    // On Windows, Crossterm changes CONSOLE_MODE through WinAPI, so ConPTY has no
    // corresponding CSI bytes to assert; the pure guard tests cover both calls.
    require(
        contains_bytes(&pty.bytes, b"\x1b[?25h"),
        "cursor visibility was not restored",
    )?;
    Ok(())
}

#[test]
fn idle_screen_emits_no_redundant_frames() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    pty.drain_for(Duration::from_millis(150))?;
    let before = pty.bytes.len();
    pty.drain_for(Duration::from_millis(250))?;
    require(
        pty.bytes.len() == before,
        "idle TUI emitted redundant frame bytes after the initial screen settled",
    )?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
fn control_j_inserts_a_visible_newline_without_submitting() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("Offline local session", Duration::from_secs(10))?;
    pty.type_text("first")?;
    pty.send(b"\x0a")?;
    pty.type_text("second")?;
    pty.wait_for_text("second", Duration::from_secs(5))?;
    let screen = terminal_screen_text(&pty.bytes);
    let first = screen
        .lines()
        .position(|line| line.contains("first"))
        .ok_or("first draft row missing")?;
    let second = screen
        .lines()
        .position(|line| line.contains("second"))
        .ok_or("second draft row missing")?;
    assert_eq!(
        second,
        first.saturating_add(1),
        "Ctrl+J must insert a real composer newline: {screen}"
    );
    assert!(
        !screen.contains("Directive committed"),
        "newline cannot submit the draft"
    );
    pty.send(b"\x03")?;
    pty.drain_for(Duration::from_millis(100))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}
