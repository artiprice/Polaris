//! Physical terminal smoke tests against the actual CLI binary through a bounded PTY.
// Test doubles and fixtures fail loudly: a panic is this test's failure signal.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use polaris_ui_cli::input::{InputReader, NormalizedInput, TerminalInput};
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
    /// Private home of the child, removed with the session.
    _home: Option<tempfile::TempDir>,
}

impl PtySession {
    fn spawn(
        state: &std::path::Path,
        workspace: &std::path::Path,
        initial: Option<&str>,
    ) -> TestResult<Self> {
        Self::spawn_with_color(state, workspace, initial, false)
    }

    fn spawn_with_color(
        state: &std::path::Path,
        workspace: &std::path::Path,
        initial: Option<&str>,
        auto_color: bool,
    ) -> TestResult<Self> {
        Self::spawn_with_profile(state, workspace, initial, auto_color, false)
    }

    fn spawn_demo(state: &std::path::Path, workspace: &std::path::Path) -> TestResult<Self> {
        Self::spawn_with_profile(state, workspace, None, false, true)
    }

    fn spawn_with_profile(
        state: &std::path::Path,
        workspace: &std::path::Path,
        initial: Option<&str>,
        auto_color: bool,
        demo: bool,
    ) -> TestResult<Self> {
        Self::spawn_with_profile_and_trace(state, workspace, initial, auto_color, demo, None)
    }

    fn spawn_with_trace(
        state: &std::path::Path,
        workspace: &std::path::Path,
        trace_path: &std::path::Path,
    ) -> TestResult<Self> {
        Self::spawn_with_profile_and_trace(state, workspace, None, false, false, Some(trace_path))
    }

    fn spawn_with_profile_and_trace(
        state: &std::path::Path,
        workspace: &std::path::Path,
        initial: Option<&str>,
        auto_color: bool,
        demo: bool,
        trace_path: Option<&std::path::Path>,
    ) -> TestResult<Self> {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_polaris"));
        command.arg("--state-dir");
        command.arg(state);
        command.arg("--workspace");
        command.arg(workspace);
        command.arg("--color");
        command.arg(if auto_color { "auto" } else { "never" });
        if let Some(trace_path) = trace_path {
            command.arg("--key-trace");
            command.arg(trace_path);
        }
        if demo {
            command.arg("--demo");
        }
        if let Some(initial) = initial {
            command.arg(initial);
        }
        command.env("TERM", "xterm-256color");
        let home = isolate_home(&mut command)?;
        if auto_color {
            command.env_remove("NO_COLOR");
            command.env_remove("COLORTERM");
        } else {
            command.env("NO_COLOR", "1");
        }
        let mut session = Self::spawn_command(command)?;
        session._home = Some(home);
        Ok(session)
    }

    fn spawn_native_probe(test_name: &str, child_marker: &str) -> TestResult<Self> {
        let mut command = CommandBuilder::new(std::env::current_exe()?);
        command.arg("--exact");
        command.arg(test_name);
        command.arg("--nocapture");
        command.arg("--test-threads=1");
        command.env(child_marker, "1");
        Self::spawn_command(command)
    }

    fn spawn_command(command: CommandBuilder) -> TestResult<Self> {
        let system = native_pty_system();
        let pair = system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
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
            _home: None,
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
                        "PTY did not render {needle:?}; captured {} bytes; first bytes: {:02x?}; raw websearch context: {:02x?}; visible output: {}",
                        self.bytes.len(),
                        self.bytes
                            .get(..self.bytes.len().min(128))
                            .map_or(self.bytes.as_slice(), |prefix| prefix),
                        byte_context(&self.bytes, b"websearch(on)", 16),
                        terminal_screen_text(&self.bytes)
                    ),
                )
                .into());
            }
            self.receive(remaining.min(Duration::from_millis(40)))?;
            if let Some(status) = self.child.try_wait()? {
                self.drain_for(Duration::from_millis(100))?;
                if contains_visible_text(&self.bytes, needle) {
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "CLI exited before rendering {needle:?} with {status}; visible output: {}",
                    terminal_screen_text(&self.bytes)
                ))
                .into());
            }
        }
    }

    fn wait_for_bytes(&mut self, needle: &[u8], timeout: Duration) -> TestResult<()> {
        let deadline = deadline_after(timeout)?;
        loop {
            if contains_bytes(&self.bytes, needle) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "PTY did not emit marker {needle:?}; captured {} bytes",
                        self.bytes.len()
                    ),
                )
                .into());
            }
            self.receive(remaining.min(Duration::from_millis(40)))?;
            if let Some(status) = self.child.try_wait()? {
                self.drain_for(Duration::from_millis(100))?;
                if contains_bytes(&self.bytes, needle) {
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "CLI exited before emitting marker {needle:?} with {status}; captured {} bytes",
                    self.bytes.len()
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
    let mut wrap_pending = false;
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
                if matches!(
                    final_byte,
                    Some('H' | 'f' | 'G' | 'd' | 'A' | 'B' | 'C' | 'D' | 'K' | 'J')
                ) {
                    wrap_pending = false;
                }
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
                    Some('X') => {
                        let end = x.saturating_add(first).min(WIDTH);
                        if let Some(cells) = cells.get_mut(y).and_then(|row| row.get_mut(x..end)) {
                            cells.fill(' ');
                        }
                    }
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
            '\r' => {
                x = 0;
                wrap_pending = false;
            }
            '\n' => {
                y = y.saturating_add(1).min(HEIGHT.saturating_sub(1));
                wrap_pending = false;
            }
            '\x08' => {
                x = x.saturating_sub(1);
                wrap_pending = false;
            }
            '\t' => x = x.saturating_add(4).min(WIDTH.saturating_sub(1)),
            character if !character.is_control() => {
                // ConPTY may emit full colored rows as one continuous stream.
                // VT wraps when the next printable cell follows the right edge;
                // intervening SGR changes do not cancel that pending wrap.
                if wrap_pending {
                    x = 0;
                    y = y.saturating_add(1).min(HEIGHT.saturating_sub(1));
                    wrap_pending = false;
                }
                if let Some(cell) = cells.get_mut(y).and_then(|row| row.get_mut(x)) {
                    *cell = character;
                }
                if x == WIDTH.saturating_sub(1) {
                    wrap_pending = true;
                } else {
                    x = x.saturating_add(1);
                }
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

fn terminal_cursor_position(bytes: &[u8]) -> Option<(usize, usize)> {
    let input = String::from_utf8_lossy(bytes);
    let mut position: Option<(usize, usize)> = None;
    for sequence in input.split("\x1b[").skip(1) {
        let Some(final_index) =
            sequence.find(|character: char| ('\x40'..='\x7e').contains(&character))
        else {
            continue;
        };
        let Some(final_byte) = sequence.chars().nth(final_index) else {
            continue;
        };
        let parameters = sequence.get(..final_index)?;
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
        let (row, column) = position.map_or((1_usize, 1_usize), |value| value);
        match final_byte {
            'H' | 'f' => position = Some((first, second)),
            'A' => position = Some((row.saturating_sub(first), column)),
            'B' => position = Some((row.saturating_add(first), column)),
            'C' => position = Some((row, column.saturating_add(first))),
            'D' => position = Some((row, column.saturating_sub(first))),
            'G' => position = Some((row, first)),
            'd' => position = Some((first, column)),
            _ => {}
        }
    }
    position
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn byte_context<'a>(haystack: &'a [u8], needle: &[u8], padding: usize) -> &'a [u8] {
    if needle.is_empty() {
        return &[];
    }
    let Some(start) = haystack
        .windows(needle.len())
        .position(|window| window == needle)
    else {
        return &[];
    };
    let context_start = start.saturating_sub(padding);
    let context_end = start
        .saturating_add(needle.len())
        .saturating_add(padding)
        .min(haystack.len());
    haystack
        .get(context_start..context_end)
        .map_or(&[], |context| context)
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

#[test]
fn terminal_parser_observes_autowrap_and_erase_characters() -> TestResult<()> {
    let text = terminal_screen_text(format!("{}B", "A".repeat(80)).as_bytes());
    let rows: Vec<_> = text.lines().collect();
    require(
        rows.first().copied() == Some("A".repeat(80).as_str()),
        "full row changed",
    )?;
    require(
        rows.get(1).copied() == Some("B"),
        "next printable cell did not wrap",
    )?;
    let text = terminal_screen_text(b"Directivepcommitted\r\x1b[10G\x1b[1X");
    require(
        text.starts_with("Directive committed"),
        "CSI X left an obsolete glyph",
    )?;
    Ok(())
}

/// Point the child's home at a private directory and drop provider keys, so a developer's
/// `~/.polaris` configuration, `.env` and keys never change what these tests observe.
fn isolate_home(command: &mut CommandBuilder) -> TestResult<tempfile::TempDir> {
    let home = tempfile::tempdir()?;
    command.env("USERPROFILE", home.path());
    command.env("HOME", home.path());
    for key in ["NEBIUS_API_KEY", "OPENROUTER_API_KEY"] {
        command.env_remove(key);
    }
    Ok(home)
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
#[cfg(windows)]
fn windows_auto_color_uses_rgb_without_colorterm() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn_with_color(state.path(), workspace.path(), None, true)?;
    pty.drain_for(Duration::from_millis(750))?;
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../@prime/evidence/ui/repair-auto-color.ansi"),
        &pty.bytes,
    )?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    require(
        contains_bytes(&pty.bytes, b"38;2;192;229;140"),
        "auto color omitted the North Star's RGB center without COLORTERM",
    )?;
    require(
        !contains_visible_text(&pty.bytes, "Offline local session"),
        "quiet welcome still included the removed persistent warning",
    )?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
fn default_terminal_launch_shows_real_welcome_at_80x24() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    // The reference welcome is quiet. Wait for its truthful footer profile
    // label rather than the removed permanent warning. All interaction and
    // lifecycle assertions below remain unchanged.
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
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
fn slash_menu_opens_help_and_escape_returns_to_editor() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    pty.type_text("/")?;
    pty.wait_for_text("/agents", Duration::from_secs(5))?;
    pty.type_text("hel")?;
    pty.wait_for_text("/help", Duration::from_secs(5))?;
    pty.send(b"\t\r")?;
    pty.wait_for_text("Commands: /help /status", Duration::from_secs(5))?;
    pty.send(b"\x1b")?;
    pty.drain_for(Duration::from_millis(150))?;
    require(
        !contains_visible_text(&pty.bytes, "Commands: /help /status"),
        "Escape left the help sheet visible",
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
    pty.drain_for(Duration::from_millis(150))?;
    require(
        !contains_visible_text(&pty.bytes, "$websearch(on)"),
        "native Ctrl+C did not clear the initial draft",
    )?;
    pty.send(b"\x03")?;
    thread::sleep(Duration::from_millis(100));
    pty.send(b"\x03")?;
    let status = pty.wait_for_exit(Duration::from_secs(3))?;
    require(status.success(), "native Ctrl+C did not exit after arming")?;
    require(
        !contains_visible_text(&pty.bytes, "Directive committed by the kernel"),
        "cleared initial prompt unexpectedly committed",
    )?;
    Ok(())
}

#[test]
fn native_conpty_console_records_preserve_dollar_and_ascii_keys() -> TestResult<()> {
    if std::env::var_os("POLARIS_NATIVE_FIXTURE_CHILD").is_some() {
        let mut input = InputReader::new()?;
        let mut output = io::stdout().lock();
        writeln!(output, "NATIVE_INPUT_FIXTURE_READY")?;
        output.flush()?;
        let deadline = deadline_after(Duration::from_secs(3))?;
        let mut saw_dollar = false;
        let mut saw_letter = false;
        while !(saw_dollar && saw_letter) && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(NormalizedInput::Key(key)) = input.poll(remaining)? {
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    match key.code {
                        KeyCode::Char('$') => saw_dollar = true,
                        KeyCode::Char('a') => saw_letter = true,
                        _ => {}
                    }
                }
            }
        }
        let success = saw_dollar && saw_letter;
        writeln!(
            output,
            "NATIVE_INPUT_FIXTURE_{}",
            if success { "PASS" } else { "FAIL" }
        )?;
        output.flush()?;
        return if success {
            Ok(())
        } else {
            Err(io::Error::other("native fixture did not deliver the expected characters").into())
        };
    }

    let mut pty = PtySession::spawn_native_probe(
        "native_conpty_console_records_preserve_dollar_and_ascii_keys",
        "POLARIS_NATIVE_FIXTURE_CHILD",
    )?;
    pty.wait_for_text("NATIVE_INPUT_FIXTURE_READY", Duration::from_secs(5))?;
    pty.send(b"$aa")?;
    pty.send(b"\x1b[13;2u")?;
    pty.wait_for_text("NATIVE_INPUT_FIXTURE_PASS", Duration::from_secs(5))?;
    let status = pty.wait_for_exit(Duration::from_secs(3))?;
    require(status.success(), "native InputReader fixture failed")?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn native_conpty_vt_sequences_preserve_editor_events() -> TestResult<()> {
    if std::env::var_os("POLARIS_NATIVE_SEQUENCE_FIXTURE_CHILD").is_some() {
        let mut input = InputReader::new()?;
        let mut output = io::stdout().lock();
        output.write_all(b"\x1b[>1uNATIVE_SEQUENCE_FIXTURE_READY")?;
        output.flush()?;

        let deadline = deadline_after(Duration::from_secs(5))?;
        let mut saw_shift_enter = false;
        let mut saw_control_enter = false;
        let mut saw_backspace = false;
        let mut saw_delete = false;
        let mut saw_middle_paste = false;
        while !(saw_shift_enter
            && saw_control_enter
            && saw_backspace
            && saw_delete
            && saw_middle_paste)
            && Instant::now() < deadline
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(input) = input.poll(remaining)? {
                match input {
                    NormalizedInput::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        match key.code {
                            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                                saw_shift_enter = true;
                            }
                            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                saw_control_enter = true;
                            }
                            KeyCode::Backspace => saw_backspace = true,
                            KeyCode::Delete => saw_delete = true,
                            _ => {}
                        }
                    }
                    NormalizedInput::Paste(payload) if payload == "middle" => {
                        saw_middle_paste = true;
                    }
                    _ => {}
                }
            }
        }
        let success =
            saw_shift_enter && saw_control_enter && saw_backspace && saw_delete && saw_middle_paste;
        output.write_all(b"\x1b[<uNATIVE_SEQUENCE_FIXTURE_")?;
        output.write_all(if success { b"PASS" } else { b"FAIL" })?;
        output.flush()?;
        return if success {
            Ok(())
        } else {
            Err(io::Error::other("native input did not preserve editor event categories").into())
        };
    }

    let mut pty = PtySession::spawn_native_probe(
        "native_conpty_vt_sequences_preserve_editor_events",
        "POLARIS_NATIVE_SEQUENCE_FIXTURE_CHILD",
    )?;
    pty.wait_for_bytes(b"NATIVE_SEQUENCE_FIXTURE_READY", Duration::from_secs(5))?;
    pty.send(b"\x1b[13;2u\x1b[13;5u\x1b[127u\x1b[3~\x1b[200~middle\x1b[201~")?;
    pty.wait_for_bytes(b"NATIVE_SEQUENCE_FIXTURE_PASS", Duration::from_secs(7))?;
    let status = pty.wait_for_exit(Duration::from_secs(3))?;
    require(status.success(), "native VT sequence fixture failed")?;
    Ok(())
}

#[test]
fn bracketed_paste_is_literal_and_never_submitted() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let trace = state.path().join("paste-input.trace");
    let mut pty = PtySession::spawn_with_trace(state.path(), workspace.path(), &trace)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    pty.send(b"\x1b[200~$websearch(on)\x1b[201~")?;
    if let Err(error) = pty.wait_for_text("$websearch(on)", Duration::from_secs(5)) {
        let categories = std::fs::read_to_string(&trace)?;
        return Err(io::Error::other(format!("{error}; input categories: {categories:?}")).into());
    }
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
    let trace = state.path().join("typed-input.trace");
    let mut pty = PtySession::spawn_with_trace(state.path(), workspace.path(), &trace)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    pty.type_text("$websearch(on)")?;
    if let Err(error) = pty.wait_for_text("$websearch(on)", Duration::from_secs(5)) {
        let categories = std::fs::read_to_string(&trace)?;
        return Err(io::Error::other(format!("{error}; input categories: {categories:?}")).into());
    }
    pty.send(b"\r\r")?;
    pty.wait_for_text("Directive committed by the kernel", Duration::from_secs(8))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn conpty_raw_mouse_mode_sequence_probe() -> TestResult<()> {
    const CHILD_MARKER: &str = "POLARIS_CONPTY_MOUSE_MODE_PROBE_CHILD";
    const START: &[u8] = b"CONPTY_MOUSE_MODE_PROBE_START";
    const ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";
    const END: &[u8] = b"CONPTY_MOUSE_MODE_PROBE_END";

    if std::env::var_os(CHILD_MARKER).is_some() {
        let mut output = io::stdout().lock();
        output.write_all(START)?;
        output.write_all(ENABLE)?;
        output.write_all(END)?;
        output.flush()?;
        return Ok(());
    }

    let mut pty =
        PtySession::spawn_native_probe("conpty_raw_mouse_mode_sequence_probe", CHILD_MARKER)?;
    let status = pty.wait_for_exit(Duration::from_secs(5))?;
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../@prime/evidence/ui/continuation-input-conpty-mouse-modes.raw"),
        &pty.bytes,
    )?;
    require(status.success(), "raw mouse mode probe child failed")?;
    require(
        contains_bytes(&pty.bytes, START),
        "ConPTY raw mouse mode probe start marker was not visible",
    )?;
    require(
        contains_bytes(&pty.bytes, END),
        "ConPTY raw mouse mode probe end marker was not visible",
    )?;
    require(
        !contains_bytes(&pty.bytes, ENABLE),
        "ConPTY raw mouse mode bytes were visible in the captured stream",
    )?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn conpty_console_mode_bits_probe() -> TestResult<()> {
    const CHILD_MARKER: &str = "POLARIS_CONPTY_MODE_BITS_PROBE_CHILD";
    const START: &[u8] = b"CONPTY_MODE_BITS_PROBE_START";
    const END: &[u8] = b"CONPTY_MODE_BITS_PROBE_END";
    const ENABLE_MOUSE_INPUT: u32 = 0x0010;
    const ENABLE_WINDOW_INPUT: u32 = 0x0008;
    const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
    const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const DISABLED_RAW_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;

    if std::env::var_os(CHILD_MARKER).is_some() {
        use polaris_ui_cli::terminal::{CrosstermOps, TerminalGuard};

        let initial = InputReader::console_input_mode_bits()?;
        let mut guard = TerminalGuard::acquire(CrosstermOps::default(), io::stdout())?;
        let raw = InputReader::console_input_mode_bits()?;
        let mut input = InputReader::new()?;
        let native = InputReader::console_input_mode_bits()?;
        let native_ready = native & ENABLE_MOUSE_INPUT != 0
            && native & ENABLE_WINDOW_INPUT != 0
            && native & ENABLE_EXTENDED_FLAGS != 0
            && native & ENABLE_VIRTUAL_TERMINAL_INPUT != 0
            && native & ENABLE_QUICK_EDIT_MODE == 0
            && native & DISABLED_RAW_FLAGS == 0;
        require(
            native_ready,
            &format!("native input mode omitted required bits: {native:#010x}"),
        )?;

        input.close()?;
        require(
            input.poll(Duration::ZERO)?.is_none(),
            "closed native input reader returned an event",
        )?;
        drop(input);
        let after_input_close = InputReader::console_input_mode_bits()?;
        require(
            after_input_close == raw,
            &format!(
                "input close did not restore its exact pre-reader snapshot: expected {raw:#010x}, observed {after_input_close:#010x}"
            ),
        )?;
        guard.close()?;
        let restored = InputReader::console_input_mode_bits()?;
        require(
            restored == initial,
            &format!(
                "terminal guard did not restore its exact original input mode: expected {initial:#010x}, observed {restored:#010x}"
            ),
        )?;

        let mut output = io::stdout().lock();
        output.write_all(START)?;
        write!(
            output,
            " initial={initial:#010x} raw={raw:#010x} native={native:#010x} after_input_close={after_input_close:#010x} restored={restored:#010x} "
        )?;
        output.write_all(END)?;
        output.flush()?;
        return Ok(());
    }

    let mut pty = PtySession::spawn_native_probe("conpty_console_mode_bits_probe", CHILD_MARKER)?;
    let status = pty.wait_for_exit(Duration::from_secs(8))?;
    require(status.success(), "console mode bit probe child failed")?;
    require(
        contains_bytes(&pty.bytes, START),
        "console mode bit probe start marker was not visible",
    )?;
    require(
        contains_bytes(&pty.bytes, END),
        "console mode bit probe end marker was not visible",
    )?;
    let evidence_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../@prime/evidence/ui")
        .join(format!(
            "mouse-repair-console-mode-probe-{}.raw",
            std::process::id()
        ));
    let mut evidence = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence_path)?;
    evidence.write_all(&pty.bytes)?;
    Ok(())
}

#[test]
fn quit_restores_terminal_modes_and_cursor() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    exit_with_quit(&mut pty)?;
    #[cfg(windows)]
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../@prime/evidence/ui/parity-input-terminal-lifecycle.raw"),
        &pty.bytes,
    )?;
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
    #[cfg(windows)]
    {
        // ConPTY filters DECSET 100x; the exact Windows mouse bytes are covered by
        // `windows_vt_mouse_and_keyboard_protocols_are_balanced` in terminal_lifecycle.
        require(
            contains_bytes(&pty.bytes, b"\x1b[>1u"),
            "CSI-u keyboard reporting was not enabled on Windows",
        )?;
        require(
            contains_bytes(&pty.bytes, b"\x1b[<u"),
            "CSI-u keyboard reporting was not restored on Windows",
        )?;
    }
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
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
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
fn enhanced_control_j_inserts_a_visible_newline_without_submitting() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    pty.type_text("first")?;
    pty.send(b"\x1b[106;5u")?;
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
    Ok(())
}

#[test]
#[cfg(windows)]
fn offline_alacritty_editor_deletes_ascii_unicode_and_middle_paste() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let trace = state.path().join("editor-input.trace");
    let mut pty = PtySession::spawn_with_trace(state.path(), workspace.path(), &trace)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[>1u"),
        "offline editor did not request enhanced key reporting",
    )?;

    pty.type_text("AéBC")?;
    pty.drain_for(Duration::from_millis(100))?;
    let before_edits = std::fs::read_to_string(&trace)?;
    pty.send(b"\x1b[D\x1b[D")?;
    pty.send(b"\x1b[200~X\x1b[201~")?;
    pty.send(b"\x1b[127u")?;
    pty.send(b"\x1b[H\x1b[3~\x1b[3~")?;
    pty.drain_for(Duration::from_millis(150))?;
    let traced_after_edits = std::fs::read_to_string(&trace)?;
    let edit_events = traced_after_edits
        .strip_prefix(&before_edits)
        .ok_or("input trace changed before the editor actions")?;
    require(
        edit_events.contains("key navigation none press\n")
            && edit_events.contains("paste\n")
            && edit_events.contains("key backspace none press\n")
            && edit_events.contains("key delete none press\n"),
        &format!("ConPTY did not produce normalized edit categories: {edit_events:?}"),
    )?;
    let edited = terminal_screen_text(&pty.bytes);
    require(
        edited.lines().any(|line| line.trim() == "> BC"),
        &format!("Backspace/Delete/middle-paste edit did not leave only BC: {edited}"),
    )?;
    pty.send(b"\x1b[F")?;
    pty.drain_for(Duration::from_millis(100))?;

    pty.send(b"\x1b[13;5u")?;
    pty.drain_for(Duration::from_millis(150))?;
    let single_row = terminal_screen_text(&pty.bytes);
    let composer_row = single_row
        .lines()
        .position(|line| line.trim() == "> BC")
        .ok_or_else(|| format!("Ctrl+Enter changed the draft row: {single_row}"))?;
    require(
        single_row
            .lines()
            .nth(composer_row + 1)
            .is_some_and(|line| line.trim().starts_with('─')),
        &format!("Ctrl+Enter changed the one-row draft: {single_row}"),
    )?;

    pty.send(b"\x1b[13;2u")?;
    pty.type_text("second")?;
    pty.wait_for_text("second", Duration::from_secs(5))?;
    let screen = terminal_screen_text(&pty.bytes);
    let first = screen
        .lines()
        .position(|line| line.contains("BC"))
        .ok_or("edited first row missing")?;
    let second = screen
        .lines()
        .position(|line| line.contains("second"))
        .ok_or("Shift+Enter second row missing")?;
    assert_eq!(second, first.saturating_add(1), "{screen}");
    pty.send(b"\x1b[97;5u\x1b[127u")?;
    exit_with_quit(&mut pty)?;
    require(
        contains_bytes(&pty.bytes, b"\x1b[<u"),
        "keyboard enhancement pop was not written on exit",
    )?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn demo_shift_enter_and_shift_up_keep_native_cursor_position_in_sync() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn_demo(state.path(), workspace.path())?;
    pty.wait_for_text("DEMO", Duration::from_secs(10))?;
    pty.type_text("first row")?;
    pty.send(b"\x1b[13;2u")?;
    pty.type_text("second row")?;
    pty.wait_for_text("second row", Duration::from_secs(5))?;
    let screen = terminal_screen_text(&pty.bytes);
    let first = screen
        .lines()
        .position(|line| line.contains("first row"))
        .ok_or("first draft row missing after CSI-u Shift+Enter")?;
    let second = screen
        .lines()
        .position(|line| line.contains("second row"))
        .ok_or("second draft row missing after CSI-u Shift+Enter")?;
    assert_eq!(second, first.saturating_add(1), "{screen}");

    let before = terminal_cursor_position(&pty.bytes)
        .ok_or("terminal output did not position the physical cursor")?;
    require(
        before.0 == second.saturating_add(1),
        &format!("physical cursor row {before:?} did not track the second composer row {second}"),
    )?;
    pty.send(b"\x1b[1;2A")?;
    pty.drain_for(Duration::from_millis(250))?;
    let after = terminal_cursor_position(&pty.bytes)
        .ok_or("terminal output omitted the physical cursor after Shift+Up")?;
    require(
        after.0 == before.0 && after.0 == second.saturating_add(1),
        &format!(
            "Shift+Up moved the physical cursor from {before:?} to {after:?}; it must stay on the draft row {second}"
        ),
    )?;
    pty.send(b"\x03")?;
    pty.drain_for(Duration::from_millis(100))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn demo_control_enter_does_not_insert_a_composer_newline() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn_demo(state.path(), workspace.path())?;
    pty.wait_for_text("DEMO", Duration::from_secs(10))?;
    pty.type_text("single row")?;
    pty.send(b"\x1b[13;5u")?;
    pty.drain_for(Duration::from_millis(250))?;
    let after = terminal_screen_text(&pty.bytes);
    require(
        after
            .lines()
            .any(|line| line.contains("single") && line.contains("row")),
        &format!("Ctrl+Enter inserted a newline: {after}"),
    )?;
    pty.send(b"\x03")?;
    pty.drain_for(Duration::from_millis(100))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn demo_prompt_is_scripted_without_creating_session_state() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn_demo(state.path(), workspace.path())?;
    pty.wait_for_text("DEMO", Duration::from_secs(10))?;
    pty.type_text("long")?;
    pty.send(b"\r")?;
    pty.wait_for_text("Retrying", Duration::from_secs(8))?;
    require(
        !contains_visible_text(&pty.bytes, "[ offline ]"),
        "demo script rendered as the offline profile",
    )?;
    pty.send(b"/quit\r\r")?;
    let status = pty.wait_for_exit(Duration::from_secs(8))?;
    require(status.success(), "demo CLI did not exit cleanly")?;
    require(
        std::fs::read_dir(state.path())?.next().is_none(),
        "demo prompt wrote session or transcript state",
    )?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn profile_picker_switches_between_offline_and_demo_and_ignores_hover_bytes() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;

    pty.send(b"\x1b[15~")?;
    pty.wait_for_text("Demo", Duration::from_secs(5))?;
    pty.send(b"\x1b[B\r")?;
    pty.wait_for_text("DEMO", Duration::from_secs(5))?;
    pty.send(b"\x1b[<35;40;24M")?;
    pty.drain_for(Duration::from_millis(150))?;
    require(
        !contains_visible_text(&pty.bytes, "35;40;24"),
        "mouse hover bytes appeared as composer text",
    )?;

    pty.send(b"\x1b[15~")?;
    pty.wait_for_text("Live", Duration::from_secs(5))?;
    pty.send(b"\x1b[A\r")?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(5))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

#[cfg(windows)]
fn click_visible_label(pty: &mut PtySession, label: &str) -> TestResult<()> {
    let screen = terminal_screen_text(&pty.bytes);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(label))
        .ok_or_else(|| format!("{label} row missing from screen: {screen}"))?;
    let byte_column = line
        .find(label)
        .ok_or_else(|| format!("{label} label disappeared while locating click target"))?;
    let column = line
        .char_indices()
        .take_while(|(byte, _)| *byte < byte_column)
        .count();
    let column = u16::try_from(column.saturating_add(1))?;
    let row = u16::try_from(row.saturating_add(1))?;
    let click = format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m");
    pty.send(click.as_bytes())?;
    Ok(())
}

#[test]
#[cfg(windows)]
fn native_conpty_mouse_click_selects_demo_profile() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let mut pty = PtySession::spawn(state.path(), workspace.path(), None)?;
    pty.wait_for_text("[ offline ]", Duration::from_secs(10))?;
    pty.send(b"\x1b[15~")?;
    pty.wait_for_text("Demo", Duration::from_secs(5))?;
    click_visible_label(&mut pty, "Demo · scripted UI")?;
    pty.wait_for_text("DEMO", Duration::from_secs(5))?;
    exit_with_quit(&mut pty)?;
    Ok(())
}

/// Loopback OpenAI-compatible provider: lists one model and streams a fixed reply.
struct FakeProvider {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<Vec<String>>>,
}

impl FakeProvider {
    const MODEL: &'static str = "fake-model";

    fn start(reply: &'static str) -> TestResult<Self> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("fake-provider".to_owned())
            .spawn(move || {
                let mut requests = Vec::new();
                while !flag.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Some(request) = Self::serve(stream, reply) {
                                requests.push(request);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
                requests
            })?;
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/", self.port)
    }

    /// Answer one request; returns its head and body for later assertions.
    fn serve(mut stream: std::net::TcpStream, reply: &str) -> Option<String> {
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8192];
        let body_start = loop {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 {
                return None;
            }
            request.extend_from_slice(buffer.get(..read)?);
            if let Some(at) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(request.get(..body_start)?).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < body_start + length {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(buffer.get(..read)?);
        }
        let text = String::from_utf8_lossy(&request).into_owned();
        let response = if text.starts_with("GET ") {
            let body = format!(
                "{{\"object\":\"list\",\"data\":[{{\"id\":\"{}\",\"object\":\"model\"}}]}}",
                Self::MODEL
            );
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            let (first, rest) = reply.split_at(reply.len() / 2);
            let chunk = |delta: &str| {
                format!(
                    "data: {{\"id\":\"c1\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"{delta}\"}},\"finish_reason\":null}}]}}\n\n"
                )
            };
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{}{}data: {{\"id\":\"c1\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":6,\"total_tokens\":18}}}}\n\ndata: [DONE]\n\n",
                chunk(first),
                chunk(rest)
            )
        };
        stream.write_all(response.as_bytes()).ok()?;
        let _flushed = stream.flush();
        Some(text)
    }

    fn finish(mut self) -> TestResult<Vec<String>> {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.thread
            .take()
            .ok_or("fake provider already stopped")?
            .join()
            .map_err(|_| io::Error::other("fake provider panicked").into())
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[test]
fn prompt_streams_a_reply_from_a_loopback_provider() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    let provider = FakeProvider::start("Hello from the fake model")?;
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_polaris"));
    command.arg("--state-dir");
    command.arg(state.path());
    command.arg("--workspace");
    command.arg(workspace.path());
    command.arg("--color");
    command.arg("never");
    command.env("TERM", "xterm-256color");
    command.env("NO_COLOR", "1");
    let home = isolate_home(&mut command)?;
    command.env("POLARIS_LOCAL__BASE_URL", provider.base_url());
    command.env("POLARIS_LOCAL__MODEL", FakeProvider::MODEL);
    command.env("POLARIS_LOCAL__CONTEXT_WINDOW", "131072");
    let mut pty = PtySession::spawn_command(command)?;
    pty._home = Some(home);
    pty.wait_for_text("[ local ]", Duration::from_secs(20))?;
    pty.type_text("say hello")?;
    pty.wait_for_text("say hello", Duration::from_secs(5))?;
    pty.send(b"\r")?;
    pty.wait_for_text("Hello from the fake model", Duration::from_secs(30))?;
    pty.wait_for_text("Worked for", Duration::from_secs(10))?;
    exit_with_quit(&mut pty)?;
    let requests = provider.finish()?;
    require(
        requests
            .iter()
            .any(|request| request.starts_with("POST ") && request.contains("say hello")),
        "the prompt reached the provider",
    )?;
    Ok(())
}

/// Live check against the first hosted provider configured in the repository `.env`.
/// The workspace is synthetic: free hosted endpoints may log prompts.
#[test]
#[ignore = "needs network and a provider key in the repository .env"]
fn live_provider_answers_a_workspace_question() -> TestResult<()> {
    let (state, workspace) = new_dirs()?;
    std::fs::write(
        workspace.path().join("notes.txt"),
        "The launch code word is PELICAN.\n",
    )?;
    let env_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_polaris"));
    command.arg("--state-dir");
    command.arg(state.path());
    command.arg("--workspace");
    command.arg(workspace.path());
    command.arg("--color");
    command.arg("never");
    command.arg("--env-file");
    command.arg(&env_file);
    command.env("TERM", "xterm-256color");
    command.env("NO_COLOR", "1");
    let home = isolate_home(&mut command)?;
    let mut pty = PtySession::spawn_command(command)?;
    pty._home = Some(home);
    pty.wait_for_text("[ openrouter ]", Duration::from_secs(30))?;
    pty.type_text("Read notes.txt and tell me the code word.")?;
    pty.send(b"\r")?;
    pty.wait_for_text("Worked for", Duration::from_secs(240))?;
    let screen = terminal_screen_text(&pty.bytes);
    println!("{screen}");
    require(screen.contains("PELICAN"), "the answer names the code word")?;
    exit_with_quit(&mut pty)?;
    Ok(())
}
