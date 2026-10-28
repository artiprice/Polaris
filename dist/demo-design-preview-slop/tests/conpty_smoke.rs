#![cfg(windows)]

//! End-to-end Windows terminal input smoke tests. These start the real polaris
//! executable under ConPTY and write terminal bytes to its input pipe. No
//! synthetic console key records or keyboard hooks are involved.

use std::{
    ffi::c_void,
    fs, io, mem,
    os::windows::io::AsRawHandle,
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

type Handle = *mut c_void;
type Hpc = Handle;
type HResult = i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_len: u16,
    reserved2: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup: StartupInfoW,
    attributes: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(
        read_pipe: *mut Handle,
        write_pipe: *mut Handle,
        attrs: *mut c_void,
        size: u32,
    ) -> i32;
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        count: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const u8,
        count: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous: *mut c_void,
        returned_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *mut c_void,
        thread_attributes: *mut c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *mut c_void,
        current_directory: *const u16,
        startup: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    fn CreatePseudoConsole(
        size: Coord,
        input: Handle,
        output: Handle,
        flags: u32,
        console: *mut Hpc,
    ) -> HResult;
    fn ClosePseudoConsole(console: Hpc);
}

const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const SHIM_EXE: &str = "POLARIS_CONPTY_SHIM_EXE";
const SHIM_TRACE: &str = "POLARIS_CONPTY_SHIM_TRACE";
const SHIM_SCENARIO: &str = "POLARIS_CONPTY_SHIM_SCENARIO";
const SHIM_COLORS: &str = "POLARIS_CONPTY_SHIM_COLORS";
const SHIM_CONFIG: &str = "POLARIS_CONPTY_SHIM_CONFIG";
static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static TRACE_ID: AtomicU64 = AtomicU64::new(0);

fn last_error(context: &str) -> io::Error {
    io::Error::other(format!("{context}: {}", io::Error::last_os_error()))
}

fn close(handle: Handle) {
    if !handle.is_null() {
        // SAFETY: callers pass a live Win32 handle owned by this test harness.
        unsafe { CloseHandle(handle) };
    }
}

fn create_pipe() -> (Handle, Handle) {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: outputs are valid pointers; null security attributes make handles private.
    assert_ne!(
        unsafe { CreatePipe(&mut read, &mut write, ptr::null_mut(), 0) },
        0,
        "{}",
        last_error("CreatePipe")
    );
    (read, write)
}

fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                slashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

fn wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct ConptyChild {
    process: Handle,
    input: Handle,
    console: Hpc,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    trace: PathBuf,
}

impl ConptyChild {
    fn spawn(scenario: &str) -> Self {
        Self::spawn_colors(scenario, "inherit")
    }
    fn spawn_colors(scenario: &str, colors: &str) -> Self {
        Self::spawn_config(scenario, colors, None)
    }
    fn spawn_config(scenario: &str, colors: &str, config: Option<&std::path::Path>) -> Self {
        let trace_dir = std::env::current_dir()
            .expect("current directory")
            .join("target")
            .join("conpty-traces");
        fs::create_dir_all(&trace_dir).expect("create workspace trace directory");
        let trace = trace_dir.join(format!(
            "polaris-conpty-{}-{}.jsonl",
            std::process::id(),
            TRACE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let (input_read, input_write) = create_pipe();
        let (output_read, output_write) = create_pipe();
        let mut console = ptr::null_mut();
        // SAFETY: pipe endpoints are live and owned here; ConPTY duplicates the
        // endpoints for its lifetime. The parent retains the write/read ends.
        let hr = unsafe {
            CreatePseudoConsole(
                Coord { x: 120, y: 40 },
                input_read,
                output_write,
                0,
                &mut console,
            )
        };
        close(input_read);
        close(output_write);
        assert!(hr >= 0, "CreatePseudoConsole HRESULT={hr:#x}");

        let mut attr_size = 0usize;
        // SAFETY: a null list requests the required allocation size.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size) };
        assert!(
            attr_size > 0,
            "InitializeProcThreadAttributeList returned no size"
        );
        let mut attr_storage = vec![0usize; attr_size.div_ceil(mem::size_of::<usize>())];
        let attr_list = attr_storage.as_mut_ptr().cast::<c_void>();
        // SAFETY: attr_storage is suitably aligned and large enough per the sizing call.
        assert_ne!(
            unsafe { InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) },
            0,
            "{}",
            last_error("InitializeProcThreadAttributeList")
        );
        // SAFETY: console is a live HPCON value and the attribute list is initialized.
        assert_ne!(
            unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    console,
                    mem::size_of::<Hpc>(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            },
            0,
            "{}",
            last_error("UpdateProcThreadAttribute(PSEUDOCONSOLE)")
        );

        let exe = env!("CARGO_BIN_EXE_polaris");
        let test_exe = std::env::current_exe().expect("test executable");
        let helper_command = [
            quote_arg(&test_exe.display().to_string()),
            quote_arg("--ignored"),
            quote_arg("--exact"),
            quote_arg("conpty_stdio_shim"),
            quote_arg("--nocapture"),
        ]
        .join(" ");
        // Cargo redirects its test runner streams. The isolated helper binds
        // CONIN$/CONOUT$ to stdio, then CreateProcessW launches polaris itself.
        let _spawn_guard = SPAWN_LOCK.lock().unwrap();
        let old_exe = std::env::var_os(SHIM_EXE);
        let old_trace = std::env::var_os(SHIM_TRACE);
        let old_scenario = std::env::var_os(SHIM_SCENARIO);
        let old_colors = std::env::var_os(SHIM_COLORS);
        let old_config = std::env::var_os(SHIM_CONFIG);
        let config = config
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| trace.with_extension("config"));
        unsafe {
            std::env::set_var(SHIM_EXE, exe);
            std::env::set_var(SHIM_TRACE, &trace);
            std::env::set_var(SHIM_SCENARIO, scenario);
            std::env::set_var(SHIM_COLORS, colors);
            std::env::set_var(SHIM_CONFIG, config);
        }
        let command = wide_nul(&helper_command);
        let application = wide_nul(&test_exe.display().to_string());
        let mut startup: StartupInfoExW = unsafe { mem::zeroed() };
        startup.startup.cb = mem::size_of::<StartupInfoExW>() as u32;
        startup.attributes = attr_list;
        let mut info: ProcessInformation = unsafe { mem::zeroed() };
        // SAFETY: command/application/cwd and startup structures remain live for
        // the call; the explicit ConPTY attribute provides the child's terminal.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_ptr() as *mut u16,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT,
                ptr::null_mut(),
                ptr::null(),
                &mut startup.startup,
                &mut info,
            )
        };
        // SAFETY: the list was initialized above and is no longer needed after CreateProcessW.
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        restore_env(SHIM_EXE, old_exe);
        restore_env(SHIM_TRACE, old_trace);
        restore_env(SHIM_SCENARIO, old_scenario);
        restore_env(SHIM_COLORS, old_colors);
        restore_env(SHIM_CONFIG, old_config);
        drop(_spawn_guard);
        if created == 0 {
            close(input_write);
            close(output_read);
            // SAFETY: console was successfully created and is owned here.
            unsafe { ClosePseudoConsole(console) };
            panic!("{}", last_error("CreateProcessW polaris"));
        }
        close(info.thread);

        let output = Arc::new(Mutex::new(Vec::new()));
        let reader_output = Arc::clone(&output);
        let output_read = output_read as usize;
        let reader = thread::spawn(move || {
            let output_read = output_read as Handle;
            let mut bytes = [0u8; 4096];
            loop {
                let mut count = 0u32;
                // SAFETY: buffer is writable and the reader owns output_read.
                let ok = unsafe {
                    ReadFile(
                        output_read,
                        bytes.as_mut_ptr(),
                        bytes.len() as u32,
                        &mut count,
                        ptr::null_mut(),
                    )
                };
                if ok == 0 || count == 0 {
                    break;
                }
                reader_output
                    .lock()
                    .unwrap()
                    .extend_from_slice(&bytes[..count as usize]);
            }
            close(output_read);
        });
        Self {
            process: info.process,
            input: input_write,
            console,
            output,
            reader: Some(reader),
            trace,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut offset = 0usize;
        while offset < bytes.len() {
            let mut written = 0u32;
            // SAFETY: input is the live host-to-ConPTY pipe; bytes stay live for this call.
            let ok = unsafe {
                WriteFile(
                    self.input,
                    bytes[offset..].as_ptr(),
                    (bytes.len() - offset).min(u32::MAX as usize) as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0, "{}", last_error("WriteFile ConPTY input"));
            assert_ne!(written, 0, "WriteFile ConPTY input made no progress");
            offset += written as usize;
        }
    }

    fn trace_text(&self) -> String {
        fs::read_to_string(&self.trace).unwrap_or_default()
    }

    fn wait_trace(&self, description: &str, check: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = self.trace_text();
            if check(&text) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {description}; trace={text:?}; output={:?}",
                String::from_utf8_lossy(&self.output_snapshot())
            );
            let mut code = 0u32;
            // SAFETY: process is this harness's live child handle.
            let wait = unsafe { WaitForSingleObject(self.process, 0) };
            if wait == WAIT_OBJECT_0 {
                unsafe { GetExitCodeProcess(self.process, &mut code) };
                panic!(
                    "polaris exited early with code {code}; waiting for {description}; trace={text:?}; ConPTY output={:?}",
                    String::from_utf8_lossy(&self.output_snapshot())
                );
            }
            assert!(wait == WAIT_TIMEOUT, "WaitForSingleObject returned {wait}");
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn output_snapshot(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    fn finish(mut self) {
        let trace = self.trace_text();
        assert!(
            trace.lines().any(|line| line.contains("\"quit\":true")),
            "caller did not request normal app exit; trace={trace:?}"
        );
        // Keep ConPTY input open until normal app exit; closing it early can
        // synthesize CONTROL_C_EXIT in the child.
        let mut wait = unsafe { WaitForSingleObject(self.process, 30_000) };
        if wait != WAIT_OBJECT_0 {
            unsafe {
                TerminateProcess(self.process, 1);
                wait = WaitForSingleObject(self.process, 2_000);
            }
        }
        close(self.input);
        self.input = ptr::null_mut();
        let mut exit = u32::MAX;
        unsafe { GetExitCodeProcess(self.process, &mut exit) };
        unsafe { ClosePseudoConsole(self.console) };
        self.console = ptr::null_mut();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        close(self.process);
        self.process = ptr::null_mut();
        assert_eq!(
            wait, WAIT_OBJECT_0,
            "polaris did not exit normally; exit={exit}"
        );
        assert_eq!(
            exit,
            0,
            "polaris exit code {exit}; trace={:?}; output={:?}",
            self.trace_text(),
            String::from_utf8_lossy(&self.output_snapshot())
        );
        assert!(
            !self.output_snapshot().is_empty(),
            "ConPTY captured no terminal output"
        );
        let output = self.output_snapshot();
        assert!(
            output.windows(5).any(|w| w == b"\x1b[>1u"),
            "ConPTY output lacked keyboard protocol push; output={:?}",
            String::from_utf8_lossy(&output)
        );
        assert!(
            output.windows(4).any(|w| w == b"\x1b[<u"),
            "ConPTY output lacked keyboard protocol pop; output={:?}",
            String::from_utf8_lossy(&output)
        );
        let trace = self.trace_text();
        assert!(!trace.trim().is_empty(), "key trace was empty");
        let _ = fs::remove_file(&self.trace);
    }
}

impl Drop for ConptyChild {
    fn drop(&mut self) {
        close(self.input);
        self.input = ptr::null_mut();
        if !self.process.is_null() {
            // SAFETY: cleanup is restricted to the child created by this test.
            unsafe {
                if WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT {
                    TerminateProcess(self.process, 1);
                    WaitForSingleObject(self.process, 2_000);
                }
            }
            close(self.process);
            self.process = ptr::null_mut();
        }
        if !self.console.is_null() {
            // SAFETY: console belongs to this harness.
            unsafe { ClosePseudoConsole(self.console) };
            self.console = ptr::null_mut();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let _ = fs::remove_file(&self.trace);
    }
}

fn has_state(line: &str, screen: &str, mode: &str, input_lines: usize, working: bool) -> bool {
    line.contains(&format!("\"screen\":\"{screen}\""))
        && line.contains(&format!("\"mode\":\"{mode}\""))
        && line.contains(&format!("\"input_lines\":{input_lines}"))
        && line.contains(&format!("\"working\":{working}"))
}

fn contains_key(
    trace: &str,
    code: &str,
    modifiers: &str,
    screen: &str,
    mode: &str,
    input_lines: usize,
    working: bool,
) -> bool {
    trace.lines().any(|line| {
        line.contains("\"event\":\"key\"")
            && line.contains(&format!("\"code\":\"{code}\""))
            && line.contains(&format!("\"modifiers\":\"{modifiers}\""))
            && has_state(line, screen, mode, input_lines, working)
    })
}

fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
    match old {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

#[test]
#[ignore = "launched only inside the isolated ConPTY session"]
fn conpty_stdio_shim() {
    let Some(exe) = std::env::var_os(SHIM_EXE) else {
        return;
    };
    let trace = std::env::var_os(SHIM_TRACE).expect("shim trace path");
    let scenario = std::env::var_os(SHIM_SCENARIO).expect("shim scenario");
    let input = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .expect("open ConPTY CONIN$");
    let output = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .expect("open ConPTY CONOUT$");
    let input_handle = input.as_raw_handle().cast::<c_void>();
    let output_handle = output.as_raw_handle().cast::<c_void>();
    // SAFETY: handles stay live through CreateProcessW and are inherited as the
    // child's standard console devices.
    unsafe {
        assert_ne!(
            SetHandleInformation(input_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT),
            0,
            "{}",
            last_error("SetHandleInformation(CONIN$)")
        );
        assert_ne!(
            SetHandleInformation(output_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT),
            0,
            "{}",
            last_error("SetHandleInformation(CONOUT$)")
        );
    }

    let exe = exe.to_string_lossy();
    let trace = trace.to_string_lossy();
    let scenario = scenario.to_string_lossy();
    let profile = std::env::var(SHIM_COLORS).unwrap_or_else(|_| "inherit".into());
    let mut arguments = vec![
        exe.as_ref(),
        "--preview",
        "--interactive",
        "--key-trace",
        trace.as_ref(),
        "--scenario",
        scenario.as_ref(),
    ];
    match profile.as_str() {
        "ansi" => arguments.extend(["--color-depth", "ansi"]),
        "force" => arguments.extend(["--color-depth", "truecolor"]),
        "never-force" => arguments.extend(["--color", "never", "--color-depth", "truecolor"]),
        "terminal" => arguments.extend(["--theme", "terminal"]),
        "always-no-color" | "always-dumb" => arguments.extend(["--color", "always"]),
        _ => {}
    }
    let command = wide_nul(
        &arguments
            .into_iter()
            .map(quote_arg)
            .collect::<Vec<_>>()
            .join(" "),
    );
    // A private environment block avoids changing the helper's or other tests'
    // environment. Windows variable names are case-insensitive.
    let mut environment = Vec::<u16>::new();
    {
        let mut values: Vec<_> = std::env::vars_os()
            .filter(|(key, _)| {
                let name = key.to_string_lossy().to_ascii_uppercase();
                name != "POLARIS_CONFIG_DIR"
                    && (profile == "inherit"
                        || !["COLORTERM", "WT_SESSION", "NO_COLOR", "TERM"]
                            .contains(&name.as_str()))
            })
            .collect();
        match profile.as_str() {
            "hint" => {
                values.push(("COLORTERM".into(), "truecolor".into()));
                values.push(("TERM".into(), "xterm-256color".into()));
            }
            "24bit" => values.push(("COLORTERM".into(), "24bit".into())),
            "wt" => values.push(("WT_SESSION".into(), "isolated-color-test".into())),
            "unknown" => values.push(("COLORTERM".into(), "unrecognized".into())),
            "no-color" | "always-no-color" => values.push(("NO_COLOR".into(), "1".into())),
            "dumb" | "always-dumb" => values.push(("TERM".into(), "dumb".into())),
            _ => {}
        }
        values.push((
            "POLARIS_CONFIG_DIR".into(),
            std::env::var_os(SHIM_CONFIG).expect("isolated config directory"),
        ));
        values.sort_by_key(|(k, _)| k.to_string_lossy().to_ascii_uppercase());
        for (key, value) in values {
            use std::os::windows::ffi::OsStrExt;
            environment.extend(key.encode_wide());
            environment.push(b'=' as u16);
            environment.extend(value.encode_wide());
            environment.push(0);
        }
        environment.push(0);
    }
    let application = wide_nul(&exe);
    let mut startup: StartupInfoW = unsafe { mem::zeroed() };
    startup.cb = mem::size_of::<StartupInfoW>() as u32;
    startup.flags = STARTF_USESTDHANDLES;
    startup.stdin = input_handle;
    startup.stdout = output_handle;
    startup.stderr = output_handle;
    let mut info: ProcessInformation = unsafe { mem::zeroed() };
    // SAFETY: console standard handles and UTF-16 arguments stay live until
    // CreateProcessW returns. This is the real CARGO_BIN_EXE_polaris process.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_ptr() as *mut u16,
            ptr::null_mut(),
            ptr::null_mut(),
            1,
            CREATE_UNICODE_ENVIRONMENT,
            if environment.is_empty() {
                ptr::null_mut()
            } else {
                environment.as_mut_ptr().cast()
            },
            ptr::null(),
            &mut startup,
            &mut info,
        )
    };
    assert_ne!(created, 0, "{}", last_error("CreateProcessW polaris"));
    close(info.thread);
    // SAFETY: only this test's polaris child is waited on and bounded.
    let wait = unsafe { WaitForSingleObject(info.process, 30_000) };
    if wait != WAIT_OBJECT_0 {
        unsafe {
            TerminateProcess(info.process, 1);
            WaitForSingleObject(info.process, 2_000);
        }
    }
    let mut exit = u32::MAX;
    unsafe {
        GetExitCodeProcess(info.process, &mut exit);
        CloseHandle(info.process);
    }
    assert_eq!(wait, WAIT_OBJECT_0, "polaris child timed out");
    assert_eq!(exit, 0, "polaris child exit code {exit}");
}

fn start_contains(trace: &str, screen: &str, mode: &str) -> bool {
    trace.lines().any(|line| {
        line.contains("\"event\":\"start\"")
            && line.contains(&format!("\"screen\":\"{screen}\""))
            && line.contains(&format!("\"mode\":\"{mode}\""))
    })
}

#[test]
fn conpty_csi_u_shift_enter_inserts_newline_without_submitting_draft() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |t| start_contains(t, "Home", "auto"));
    child.send(b"plaintextdraft");
    child.wait_trace("typed one-line draft", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("\"input_chars\":14")
                && line.contains("\"screen\":\"Home\"")
        })
    });
    child.send(b"\x1b[13;2u");
    let trace = child.wait_trace("CSI-u Shift+Enter newline", |t| {
        contains_key(t, "Enter", "KeyModifiers(SHIFT)", "Home", "auto", 2, false)
    });
    assert!(
        !trace.contains("plaintextdraft"),
        "key trace leaked draft text: {trace:?}"
    );
    child.send(b"\x03");
    child.wait_trace("Ctrl+C clears the draft", |t| {
        t.lines()
            .any(|line| line.contains("\"input_chars\":0") && line.contains("\"quit\":false"))
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed Ctrl+C exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_shift_tab_encodings_toggle_mode_in_both_directions() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |t| start_contains(t, "Home", "auto"));
    child.send(b"\x1b[Z");
    child.wait_trace("CSI Z BackTab toggles plan", |t| {
        contains_key(
            t,
            "BackTab",
            "KeyModifiers(SHIFT)",
            "Home",
            "plan",
            1,
            false,
        )
    });
    child.send(b"\x1b[9;2u");
    child.wait_trace("CSI-u Shift+Tab toggles auto", |t| {
        contains_key(
            t,
            "BackTab",
            "KeyModifiers(SHIFT)",
            "Home",
            "auto",
            1,
            false,
        )
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed Ctrl+C exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_escape_and_return_close_status_help_stats_and_apply_model() {
    for (scenario, key_bytes, code) in [
        ("status", b"\x1b".as_slice(), "Esc"),
        ("status", b"\x1b[13;1u".as_slice(), "Enter"),
        ("models", b"\x1b[13;1u".as_slice(), "Enter"),
        ("help", b"\x1b".as_slice(), "Esc"),
        ("help", b"\x1b[13;1u".as_slice(), "Enter"),
        ("stats", b"\x1b".as_slice(), "Esc"),
        ("stats", b"\x1b[13;1u".as_slice(), "Enter"),
    ] {
        let child = ConptyChild::spawn(scenario);
        let initial = match scenario {
            "status" => "Status",
            "models" => "Models",
            "help" => "Help",
            "stats" => "Stats",
            _ => unreachable!(),
        };
        child.wait_trace("initial picker state", |t| {
            start_contains(t, initial, "auto")
        });
        if scenario == "models" {
            child.send(b"\x1b[B");
            child.wait_trace("model selection Down arrow", |t| {
                contains_key(t, "Down", "KeyModifiers(0x0)", "Models", "auto", 1, false)
            });
        }
        child.send(key_bytes);
        child.wait_trace("key closes overlay", |t| {
            t.lines().any(|line| {
                line.contains("\"event\":\"key\"")
                    && line.contains(&format!("\"code\":\"{code}\""))
                    && line.contains("\"modifiers\":\"KeyModifiers(0x0)\"")
                    && line.contains("\"screen\":\"Home\"")
            })
        });
        if scenario == "models" {
            assert!(
                child
                    .trace_text()
                    .contains("\"active_model\":\"nvidia/nvidia-nemotron-nano-9b-v2\""),
                "Return did not apply the selected model: {:?}",
                child.trace_text()
            );
        }
        child.send(b"\x03\x03");
        child.wait_trace("confirmed Ctrl+C exit", |t| {
            t.lines().any(|line| line.contains("\"quit\":true"))
        });
        child.finish();
    }
}

#[test]
fn conpty_long_bracketed_paste_becomes_one_attachment_chip() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |t| start_contains(t, "Home", "auto"));

    let mut paste = b"\x1b[200~".to_vec();
    paste.extend(vec![b'p'; 4096]);
    paste.extend_from_slice(b"\x1b[201~");
    let paste_started = Instant::now();
    child.send(&paste);
    let trace = child.wait_trace("long bracketed paste becomes one chip", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"paste\"")
                && line.contains("\"screen\":\"Home\"")
                && line.contains("\"input_lines\":1")
                && line.contains("\"input_chars\":27")
                && line.contains("\"attachment_count\":1")
                && line.contains("\"quit\":false")
        })
    });
    eprintln!(
        "4 KiB bracketed paste attachment latency: {:?}",
        paste_started.elapsed()
    );
    assert!(
        !trace.contains(&"p".repeat(512)),
        "trace exposed pasted text"
    );

    child.send(b"\x03");
    child.wait_trace("Ctrl+C clears pasted attachment", |t| {
        t.lines().any(|line| {
            line.contains("KeyModifiers(CONTROL)")
                && line.contains("\"input_chars\":0")
                && line.contains("\"attachment_count\":0")
        })
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed Ctrl+C exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_sgr_mouse_opens_attachment_and_footer_model_picker() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |t| start_contains(t, "Home", "auto"));

    let mut paste = b"\x1b[200~".to_vec();
    paste.extend(vec![b'p'; 4096]);
    paste.extend_from_slice(b"\x1b[201~");
    child.send(&paste);
    child.wait_trace("long paste attachment chip", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"paste\"")
                && line.contains("\"input_chars\":27")
                && line.contains("\"attachment_count\":1")
        })
    });

    // The prompt occupies row 37 at the 120x40 ConPTY size; [Pasted Content 4096 chars] starts
    // at column 7 (one-based). Clicking its label opens the rich attachment preview.
    child.send(b"\x1b[<0;8;37M\x1b[<0;8;37m");
    child.wait_trace("SGR click opens attachment preview", |t| {
        t.lines()
            .any(|line| line.contains("\"event\":\"mouse\"") && line.contains("\"preview\":true"))
    });
    child.send(b"\x1b");
    child.wait_trace("Escape closes attachment preview", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("\"code\":\"Esc\"")
                && line.contains("\"screen\":\"Home\"")
        })
    });

    // Footer model control is at row 40 for this terminal size.
    child.send(b"\x1b[<0;95;40M\x1b[<0;95;40m");
    child.wait_trace("footer click opens Models", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"mouse\"") && line.contains("\"screen\":\"Models\"")
        })
    });
    child.send(b"\x1b[<65;60;28M");
    child.wait_trace("SGR wheel moves model selection", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"mouse\"")
                && line.contains("\"screen\":\"Models\"")
                && line.contains("\"selected\":1")
        })
    });
    child.send(b"\r");
    child.wait_trace("Return applies wheel-selected model", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("\"code\":\"Enter\"")
                && line.contains("\"screen\":\"Home\"")
                && line.contains("\"active_model\":\"nvidia/nvidia-nemotron-nano-9b-v2\"")
        })
    });

    child.send(b"\x03");
    child.wait_trace("Ctrl+C clears pasted attachment", |t| {
        t.lines().any(|line| {
            line.contains("KeyModifiers(CONTROL)")
                && line.contains("\"input_chars\":0")
                && line.contains("\"attachment_count\":0")
        })
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed Ctrl+C exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_arrow_and_bracketed_paste_do_not_submit_input() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |t| start_contains(t, "Home", "auto"));

    child.send(b"\x1b[B");
    child.wait_trace("ordinary Down arrow", |t| {
        contains_key(t, "Down", "KeyModifiers(0x0)", "Home", "auto", 1, false)
    });

    child.send(b"\x1b[200~pastepayload\x1b[201~");
    child.wait_trace("bracketed paste inserted without submitting", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"paste\"")
                && line.contains("\"screen\":\"Home\"")
                && line.contains("\"input_lines\":1")
                && line.contains("\"input_chars\":12")
                && line.contains("\"quit\":false")
        })
    });
    let trace = child.trace_text();
    assert!(
        !trace.lines().any(|line| {
            line.contains("\"event\":\"key\"") && line.contains("\"code\":\"Enter\"")
        }),
        "pasting unexpectedly submitted a key event: {trace:?}"
    );

    child.send(b"\x03");
    child.wait_trace("Ctrl+C clears pasted draft", |t| {
        t.lines()
            .any(|line| line.contains("\"input_chars\":0") && line.contains("\"quit\":false"))
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed Ctrl+C exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_file_completion_and_combined_directives_submit_as_one_request() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home state", |trace| {
        start_contains(trace, "Home", "auto")
    });
    child.send(b"@src/ap");
    child.wait_trace("workspace file suggestion rendered", |_| {
        let output = child.output_snapshot();
        output
            .windows(b"src/app.rs".len())
            .any(|part| part == b"src/app.rs")
    });
    child.send(b"\t");
    child.wait_trace("Tab completes only the file token", |trace| {
        trace.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("\"code\":\"Tab\"")
                && line.contains("\"input_chars\":12")
                && line.contains("\"working\":false")
        })
    });
    child.send(b"/plan /goal improve");
    child.wait_trace("both directives stay in the editable draft", |trace| {
        trace.lines().any(|line| {
            line.contains("\"input_chars\":31")
                && line.contains("\"mode\":\"auto\"")
                && line.contains("\"working\":false")
        })
    });
    child.send(b"\r");
    child.wait_trace("Return sends one planning request", |trace| {
        trace.lines().any(|line| {
            line.contains("\"code\":\"Enter\"")
                && line.contains("\"mode\":\"plan\"")
                && line.contains("\"working\":true")
                && line.contains("\"submission_count\":1")
        })
    });
    child.send(b"\x03");
    child.wait_trace("interrupt submitted preview work", |trace| {
        trace.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("KeyModifiers(CONTROL)")
                && line.contains("\"mode\":\"plan\"")
                && line.contains("\"working\":false")
                && line.contains("\"submission_count\":1")
        })
    });
    child.send(b"\x03\x03");
    child.wait_trace("confirmed exit", |trace| {
        trace.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_shift_arrows_change_active_effort_in_home_without_changing_draft() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial effort high", |trace| {
        start_contains(trace, "Home", "auto") && trace.contains("\"effort\":\"high\"")
    });
    child.send(b"keepdraft");
    child.wait_trace("draft entered", |trace| {
        trace
            .lines()
            .any(|line| line.contains("\"event\":\"key\"") && line.contains("\"input_chars\":9"))
    });
    for (bytes, code, effort) in [
        (b"\x1b[1;2A".as_slice(), "Up", "xhigh"),
        (b"\x1b[1;2B".as_slice(), "Down", "high"),
        (b"\x1b[57352;2u".as_slice(), "Up", "xhigh"),
        (b"\x1b[57353;2u".as_slice(), "Down", "high"),
        (b"\x1b[1;2B".as_slice(), "Down", "medium"),
        (b"\x1b[1;2B".as_slice(), "Down", "low"),
        (b"\x1b[1;2A".as_slice(), "Up", "medium"),
        (b"\x1b[1;2A".as_slice(), "Up", "high"),
        (b"\x1b[1;2A".as_slice(), "Up", "xhigh"),
        (b"\x1b[1;2A".as_slice(), "Up", "max"),
        (b"\x1b[1;2B".as_slice(), "Down", "xhigh"),
    ] {
        let from = child.trace_text().len();
        child.send(bytes);
        let trace = child.wait_trace("Shift arrow changes active effort", |trace| {
            trace.get(from..).is_some_and(|tail| {
                tail.lines().any(|line| {
                    line.contains("\"event\":\"key\"")
                        && line.contains(&format!("\"code\":\"{code}\""))
                        && line.contains("\"modifiers\":\"KeyModifiers(SHIFT)\"")
                        && line.contains("\"screen\":\"Home\"")
                        && line.contains(&format!("\"effort\":\"{effort}\""))
                        && line.contains("\"input_chars\":9")
                        && line.contains("\"input_lines\":1")
                        && line.contains("\"submission_count\":0")
                })
            })
        });
        assert!(
            !trace.contains("keepdraft"),
            "trace must not expose the typed draft"
        );
    }
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |trace| {
        trace.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_double_click_hold_and_drag_selects_whole_words_in_the_composer() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    // Queue typing and the double click together: the mouse must see the new
    // draft even when both arrive in one input batch, before an ordinary repaint.
    // One-based SGR coordinates: the first word starts at column 7, row 37.
    // Keep the second press held to avoid writing the OS clipboard.
    child.send(b"alpha beta gamma delta\x1b[<0;9;37M\x1b[<0;9;37m\x1b[<0;9;37M");
    child.wait_trace("double click selects alpha", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"mouse\"") && line.contains("\"selection_bytes\":5")
        })
    });
    child.send(b"\x1b[<32;20;37M");
    child.wait_trace("held drag selects alpha beta gamma", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"mouse\"") && line.contains("\"selection_bytes\":16")
        })
    });
    child.send(b"\x7f");
    child.wait_trace("Backspace deletes exactly the selected words", |t| {
        t.lines().any(|line| {
            line.contains("\"code\":\"Backspace\"")
                && line.contains("\"input_chars\":6")
                && line.contains("\"selection_bytes\":0")
        })
    });
    child.send(b"\x1b[<0;20;37m\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_control_o_toggles_inline_details_without_opening_a_popup() {
    let child = ConptyChild::spawn("conversation");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    child.send(b"unsent draft");
    child.wait_trace("draft typed", |t| {
        t.lines().any(|line| line.contains("\"input_chars\":12"))
    });
    for expanded in [true, false, true] {
        let previous = child.trace_text().lines().count();
        child.send(b"\x0f");
        child.wait_trace("Ctrl+O inline expansion", |t| {
            t.lines().skip(previous).any(|line| {
                line.contains("\"event\":\"key\"")
                    && line.contains("\"modifiers\":\"KeyModifiers(CONTROL)\"")
                    && line.contains("\"screen\":\"Home\"")
                    && line.contains("\"input_chars\":12")
                    && line.contains(&format!("\"tool_details_expanded\":{expanded}"))
            })
        });
    }
    assert!(!child.trace_text().contains("\"screen\":\"Transcript\""));
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_commands_and_file_completion_work_during_a_response() {
    let child = ConptyChild::spawn("working");
    child.wait_trace("initial working Home", |t| {
        start_contains(t, "Home", "auto") && t.contains("\"working\":true")
    });
    child.send(b"/usage\r");
    child.wait_trace("Usage opens while response continues", |t| {
        t.lines()
            .any(|line| line.contains("\"screen\":\"Usage\"") && line.contains("\"working\":true"))
    });
    child.send(b"\x1b");
    child.wait_trace("Escape returns to working Home", |t| {
        contains_key(t, "Esc", "KeyModifiers(0x0)", "Home", "auto", 1, true)
    });
    child.send(b"@src/ap");
    child.wait_trace("workspace suggestion while working", |_| {
        child
            .output_snapshot()
            .windows(b"src/app.rs".len())
            .any(|s| s == b"src/app.rs")
    });
    child.send(b"\t");
    child.wait_trace("Tab inserts file mention without interrupting", |t| {
        t.lines().any(|line| {
            line.contains("\"code\":\"Tab\"")
                && line.contains("\"input_chars\":12")
                && line.contains("\"working\":true")
        })
    });
    child.send(b"\x03\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}
#[test]
fn conpty_mouse_title_click_expands_and_collapses_only_one_activity() {
    use polaris::{
        app::App,
        interaction::{HitAction, UiMap},
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = App::default();
    app.load_scenario("conversation").unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();

    let child = ConptyChild::spawn("conversation");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    for expected in [1, 0] {
        terminal
            .draw(|f| map = ui::render_interactive(f, &app))
            .unwrap();
        let rect = map
            .targets
            .iter()
            .find(|t| t.action == HitAction::ToggleTool(2))
            .expect("last tool title remains visible")
            .rect;
        let previous = child.trace_text().lines().count();
        child.send(
            format!(
                "\x1b[<0;{};{}M\x1b[<0;{};{}m",
                rect.x + 2,
                rect.y + 1,
                rect.x + 2,
                rect.y + 1
            )
            .as_bytes(),
        );
        child.wait_trace(&format!("individual title toggle to {expected}"), |t| {
            t.lines().skip(previous).any(|line| {
                line.contains("\"event\":\"mouse\"")
                    && line.contains(&format!("\"expanded_tool_count\":{expected}"))
                    && line.contains("\"tool_details_expanded\":false")
            })
        });
        app.toggle_tool_details(2);
    }
    child.send(b"\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_completed_response_shows_work_duration_and_clock() {
    let child = ConptyChild::spawn("working");
    child.wait_trace("working Home", |t| {
        start_contains(t, "Home", "auto") && t.contains("\"working\":true")
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let output = child.output_snapshot();
        if terminal_screen(&output, 120, 40).contains("Worked for") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "completed-work summary did not render"
        );
        thread::sleep(Duration::from_millis(10));
    }
    child.send(b"x");
    child.wait_trace("work completed and composer remains editable", |t| {
        t.lines()
            .any(|line| line.contains("\"input_chars\":1") && line.contains("\"working\":false"))
    });
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_escape_interrupts_work_without_clearing_the_draft() {
    let child = ConptyChild::spawn("working");
    child.wait_trace("working Home", |t| {
        start_contains(t, "Home", "auto") && t.contains("\"working\":true")
    });
    child.send(b"keep draft");
    child.wait_trace("draft typed", |t| {
        t.lines()
            .any(|line| line.contains("\"input_chars\":10") && line.contains("\"working\":true"))
    });
    child.send(b"\x1b");
    child.wait_trace("Escape interrupts and preserves draft", |t| {
        t.lines().any(|line| {
            line.contains("\"event\":\"key\"")
                && line.contains("\"code\":\"Esc\"")
                && line.contains("\"screen\":\"Home\"")
                && line.contains("\"working\":false")
                && line.contains("\"input_chars\":10")
        })
    });
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_history_arrows_recall_and_restore_draft_at_multiline_boundaries() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    for (text, count) in [(b"older".as_slice(), 1), (b"newer prompt".as_slice(), 2)] {
        let previous = child.trace_text().lines().count();
        child.send(text);
        child.send(b"\r");
        child.wait_trace("prompt submitted", |t| {
            t.lines().skip(previous).any(|line| {
                line.contains(&format!("\"submission_count\":{count}"))
                    && line.contains("\"working\":true")
            })
        });
        let previous = child.trace_text().lines().count();
        child.send(b"\x1b");
        child.wait_trace("interrupt before next draft", |t| {
            t.lines()
                .skip(previous)
                .any(|line| line.contains("\"code\":\"Esc\"") && line.contains("\"working\":false"))
        });
    }
    child.send(b"draft\x1b[13;2uend"); // Shift+Enter is a real new line.
    child.wait_trace("multiline draft", |t| {
        t.lines()
            .any(|line| line.contains("\"input_chars\":9") && line.contains("\"input_lines\":2"))
    });
    for (bytes, chars, lines) in [
        (b"\x1b[A".as_slice(), 9, 2), // move to first row without recalling
        (b"\x1b[A".as_slice(), 12, 1),
        (b"\x1b[A".as_slice(), 5, 1),
        (b"\x1b[B".as_slice(), 12, 1),
        (b"\x1b[B".as_slice(), 9, 2), // restore unsent multiline draft
        (b"\x1b[B".as_slice(), 9, 2), // move back to its last row
    ] {
        let previous = child.trace_text().lines().count();
        child.send(bytes);
        child.wait_trace("history boundary transition", |t| {
            t.lines().skip(previous).any(|line| {
                line.contains("\"event\":\"key\"")
                    && line.contains(&format!("\"input_chars\":{chars}"))
                    && line.contains(&format!("\"input_lines\":{lines}"))
                    && line.contains("\"submission_count\":2")
            })
        });
    }
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_mouse_wheel_navigates_completion_without_scrolling_the_conversation() {
    use polaris::{
        app::App,
        interaction::{HitAction, UiMap},
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = App::default();
    app.input.set("/");
    let count = app.suggestions().len();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();
    let rect = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Suggestion(0))
        .unwrap()
        .rect;
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    child.send(b"/");
    child.wait_trace("completion opened", |t| {
        t.lines().any(|line| line.contains("\"input_chars\":1"))
    });
    for (button, expected) in [(64, 0), (65, 1), (65, 2), (64, 1)] {
        let previous = child.trace_text().lines().count();
        child.send(format!("\x1b[<{button};{};{}M", rect.x + 2, rect.y + 1).as_bytes());
        child.wait_trace("wheel stays in completion", |t| {
            t.lines().skip(previous).any(|line| {
                line.contains("\"event\":\"mouse\"")
                    && line.contains(&format!("\"selected\":{expected}"))
                    && line.contains("\"scroll\":0")
                    && line.contains("\"input_chars\":1")
            })
        });
    }
    let previous = child.trace_text().lines().count();
    for _ in 0..count + 2 {
        child.send(format!("\x1b[<65;120;{}M", rect.y).as_bytes());
    } // full-width blank top margin
    child.wait_trace("wheel reaches last command and clamps", |t| {
        t.lines().skip(previous).any(|line| {
            line.contains("\"event\":\"mouse\"")
                && line.contains(&format!("\"selected\":{}", count - 1))
                && line.contains("\"scroll\":0")
        })
    });
    let previous = child.trace_text().lines().count();
    child.send(b"\x1b[A");
    child.wait_trace("keyboard continues after wheel", |t| {
        t.lines().skip(previous).any(|line| {
            line.contains("\"code\":\"Up\"")
                && line.contains(&format!("\"selected\":{}", count - 2))
        })
    });
    child.send(b"\x03\x03\x03");
    child.wait_trace("confirmed exit", |t| {
        t.lines().any(|line| line.contains("\"quit\":true"))
    });
    child.finish();
}

#[test]
fn conpty_placeholder_persistent_dollar_setting_and_goal_status_follow_real_input() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("initial Home", |t| start_contains(t, "Home", "auto"));
    child.wait_trace("placeholder rendered", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40)
            .contains("What's on your mind for Polaris?")
    });
    let previous = child.trace_text().lines().count();
    child.send(b"$websearch\r");
    child.wait_trace("bare toggle consumes one Return without response", |t| {
        t.lines().skip(previous).any(|l| {
            l.contains("\"code\":\"Enter\"")
                && l.contains("\"input_chars\":0")
                && l.contains("\"submission_count\":0")
                && l.contains("\"working\":false")
        })
    });
    child.send(b"/status\r");
    child.wait_trace("Status opens", |t| t.contains("\"screen\":\"Status\""));
    child.send(b"\x1b[6~\x1b[6~\x1b[6~");
    child.wait_trace("saved setting visible", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("$websearch=on")
    });
    let previous = child.trace_text().lines().count();
    child.send(b"\x1b");
    child.wait_trace("Status closes", |t| {
        t.lines()
            .skip(previous)
            .any(|l| l.contains("\"screen\":\"Home\""))
    });
    child.send(b"$websearch $subagents $rag=[rag1, rag2] $skills=[skill1, skill2]\x1b[13;2u/goal /plan hello\r");
    child.wait_trace("goal request submitted", |t| {
        t.lines().any(|l| {
            l.contains("\"working\":true")
                && l.contains("\"submission_count\":1")
                && l.contains("\"mode\":\"plan\"")
        })
    });
    child.wait_trace("active goal status rendered", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("goal active (")
    });
    let previous = child.trace_text().lines().count();
    child.send(b"\x1b");
    child.wait_trace("Escape pauses goal response", |t| {
        t.lines()
            .skip(previous)
            .any(|l| l.contains("\"code\":\"Esc\"") && l.contains("\"working\":false"))
    });
    child.wait_trace("paused goal status rendered", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("paused")
    });
    child.send(b"/status\r");
    child.wait_trace("Status exposes mixed-prompt session settings", |t| {
        t.lines().any(|l| l.contains("\"screen\":\"Status\""))
    });
    child.send(b"\x1b[6~\x1b[6~\x1b[6~");
    child.wait_trace("session skills visible", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("$skills=[skill1, skill2]")
    });
    child.wait_trace("mixed RAG setting visible", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("$rag=[rag1, rag2]")
    });
    child.send(b"\x1b");
    child.send(b"\x03\x03");
    child.wait_trace("confirmed exit", |t| t.contains("\"quit\":true"));
    child.finish();
}

/// Reconstruct the terminal cells used by ConPTY redraws. Do not confuse a
/// diff split by CSI cursor moves with missing or contiguous screen text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CapturedColor {
    #[default]
    Default,
    Indexed(u16),
    Rgb(u8, u8, u8),
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct StyledCell {
    symbol: String,
    fg: CapturedColor,
    bg: CapturedColor,
}
impl Default for StyledCell {
    fn default() -> Self {
        Self::blank(CapturedColor::Default, CapturedColor::Default)
    }
}
impl StyledCell {
    fn blank(fg: CapturedColor, bg: CapturedColor) -> Self {
        Self {
            symbol: " ".into(),
            fg,
            bg,
        }
    }
}
fn apply_sgr(nums: &[usize], fg: &mut CapturedColor, bg: &mut CapturedColor) {
    let mut i = 0;
    while i < nums.len() {
        let n = nums[i];
        match n {
            0 => {
                *fg = CapturedColor::Default;
                *bg = CapturedColor::Default;
            }
            39 => *fg = CapturedColor::Default,
            49 => *bg = CapturedColor::Default,
            30..=37 => *fg = CapturedColor::Indexed((n - 30) as u16),
            90..=97 => *fg = CapturedColor::Indexed((n - 90 + 8) as u16),
            40..=47 => *bg = CapturedColor::Indexed((n - 40) as u16),
            100..=107 => *bg = CapturedColor::Indexed((n - 100 + 8) as u16),
            38 | 48 => {
                let target = if n == 38 { &mut *fg } else { &mut *bg };
                if nums.get(i + 1) == Some(&2) && i + 4 < nums.len() {
                    *target =
                        CapturedColor::Rgb(nums[i + 2] as u8, nums[i + 3] as u8, nums[i + 4] as u8);
                    i += 4;
                } else if nums.get(i + 1) == Some(&5) && i + 2 < nums.len() {
                    *target = CapturedColor::Indexed(nums[i + 2] as u16);
                    i += 2;
                }
            }
            _ => {}
        }
        i += 1;
    }
}
fn terminal_screen(bytes: &[u8], width: usize, height: usize) -> String {
    styled_terminal_screen(bytes, width, height)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|c| c.symbol)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
struct TerminalState {
    cells: Vec<Vec<StyledCell>>,
    cursor: (usize, usize),
    visible: bool,
    reveals: Vec<(usize, usize)>,
}
fn styled_terminal_screen(bytes: &[u8], width: usize, height: usize) -> Vec<Vec<StyledCell>> {
    terminal_state(bytes, width, height).cells
}
fn terminal_state(bytes: &[u8], width: usize, height: usize) -> TerminalState {
    use unicode_width::UnicodeWidthChar;
    let mut cells = vec![vec![StyledCell::default(); width]; height];
    let (mut fg, mut bg) = (CapturedColor::Default, CapturedColor::Default);
    let (mut x, mut y) = (0usize, 0usize);
    let mut visible = true;
    let mut reveals = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.next() != Some('[') {
                continue;
            }
            let mut params = String::new();
            let mut final_ch = None;
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    final_ch = Some(c);
                    break;
                }
                params.push(c);
            }
            if params == "?25" {
                if final_ch == Some('h') {
                    visible = true;
                    reveals.push((x, y));
                } else if final_ch == Some('l') {
                    visible = false;
                }
            }
            if params.starts_with(['?', '>', '<']) {
                continue;
            }
            let nums = params
                .split(';')
                .map(|s| s.parse::<usize>().unwrap_or(0))
                .collect::<Vec<_>>();
            let n = nums.first().copied().unwrap_or(0);
            let step = n.max(1);
            match final_ch {
                Some('m') => apply_sgr(&nums, &mut fg, &mut bg),
                Some('H' | 'f') => {
                    y = step.saturating_sub(1).min(height - 1);
                    x = nums
                        .get(1)
                        .copied()
                        .unwrap_or(1)
                        .max(1)
                        .saturating_sub(1)
                        .min(width - 1);
                }
                Some('A') => y = y.saturating_sub(step),
                Some('B') => y = y.saturating_add(step).min(height - 1),
                Some('C') => x = x.saturating_add(step).min(width - 1),
                Some('D') => x = x.saturating_sub(step),
                Some('G') => x = step.saturating_sub(1).min(width - 1),
                Some('X') => {
                    for cell in cells[y].iter_mut().skip(x.min(width)).take(step) {
                        *cell = StyledCell::blank(fg, bg);
                    }
                }
                Some('K') => {
                    let range = match n {
                        1 => 0..x.saturating_add(1).min(width),
                        2 => 0..width,
                        _ => x.min(width)..width,
                    };
                    for i in range {
                        cells[y][i] = StyledCell::blank(fg, bg);
                    }
                }
                Some('J') => {
                    for (row_index, row) in cells.iter_mut().enumerate() {
                        for (col, cell) in row.iter_mut().enumerate() {
                            if n == 2
                                || (n == 0 && (row_index > y || (row_index == y && col >= x)))
                                || (n == 1 && (row_index < y || (row_index == y && col <= x)))
                            {
                                *cell = StyledCell::blank(fg, bg);
                            }
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match ch {
            '\r' => {
                x = 0;
                continue;
            }
            '\n' => {
                y += 1;
            }
            '\x08' => {
                x = x.saturating_sub(1);
                continue;
            }
            c if c.is_control() => continue,
            c => {
                let size = c.width().unwrap_or(0);
                if size == 0 {
                    continue;
                }
                if x + size > width {
                    x = 0;
                    y += 1;
                }
                if y >= height {
                    cells.remove(0);
                    cells.push(vec![StyledCell::blank(fg, bg); width]);
                    y = height - 1;
                }
                cells[y][x] = StyledCell {
                    symbol: c.to_string(),
                    fg,
                    bg,
                };
                for tail in 1..size {
                    cells[y][x + tail] = StyledCell {
                        symbol: String::new(),
                        fg,
                        bg,
                    };
                }
                x += size;
            }
        }
        if y >= height {
            cells.remove(0);
            cells.push(vec![StyledCell::blank(fg, bg); width]);
            y = height - 1;
        }
    }
    TerminalState {
        cells,
        cursor: (x, y),
        visible,
        reveals,
    }
}
#[test]
fn terminal_screen_reconstructs_split_values_and_rejects_erased_stale_text() {
    let text = b"\x1b[2J\x1b[2;3H$rag=[rag1,\x1b[1C\x1b[?25hrag2]";
    assert!(terminal_screen(text, 40, 4).contains("$rag=[rag1, rag2]"));
    let mut erased = text.to_vec();
    erased.extend_from_slice(b"\x1b[2;3H\x1b[K$rag=[]");
    let screen = terminal_screen(&erased, 40, 4);
    assert!(screen.contains("$rag=[]"));
    assert!(!screen.contains("rag1"));
}

#[test]
fn styled_decoder_keeps_rgb_indexed_resets_and_erase_attributes() {
    let cells=styled_terminal_screen(b"\x1b[38;2;36;208;70m\x1b[48;2;28;28;28mA\x1b[39mB\x1b[45mC\x1b[0mD\x1b[1;1H\x1b[48;5;234m\x1b[X",8,2);
    assert_eq!(cells[0][0].symbol, " ");
    assert_eq!(cells[0][0].bg, CapturedColor::Indexed(234));
    assert_eq!(cells[0][1].fg, CapturedColor::Default);
    assert_eq!(cells[0][1].bg, CapturedColor::Rgb(28, 28, 28));
    assert_eq!(cells[0][2].bg, CapturedColor::Indexed(5));
    assert_eq!(cells[0][3].bg, CapturedColor::Default);
    let c = styled_terminal_screen(b"\x1b[38;2;36;208;70mA", 2, 1);
    assert_eq!(c[0][0].fg, CapturedColor::Rgb(36, 208, 70));
}
fn color_capture(profile: &str, scenario: &str, depth: &str) -> Vec<Vec<StyledCell>> {
    let child = ConptyChild::spawn_colors(scenario, profile);
    child.wait_trace("resolved startup colors", |t| {
        t.lines().any(|l| {
            l.contains("\"event\":\"start\"") && l.contains(&format!("\"color_depth\":\"{depth}\""))
        })
    });
    child.wait_trace("complete initial frame", |_| {
        let bytes = child.output_snapshot();
        let state = terminal_state(&bytes, 120, 40);
        state.visible
            && state.cursor == (6, 36)
            && terminal_screen(&bytes, 120, 40).contains("auto mode on")
    });
    let bytes = child.output_snapshot();
    let cells = styled_terminal_screen(&bytes, 120, 40);
    // Store raw transport bytes as evidence; never capture user input.
    fs::write(
        child
            .trace
            .with_extension(format!("{profile}-{scenario}.ansi")),
        &bytes,
    )
    .unwrap();
    fs::write(
        child
            .trace
            .with_extension(format!("{profile}-{scenario}.colors.jsonl")),
        child.trace_text(),
    )
    .unwrap();
    child.send(b"\x03\x03");
    child.wait_trace("color test normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
    cells
}
#[test]
fn conpty_default_rgb_cells_match_with_missing_alacritty_and_windows_terminal_hints() {
    for scenario in ["welcome", "conversation"] {
        let plain = color_capture("plain", scenario, "truecolor");
        for hint in ["hint", "24bit", "wt", "unknown"] {
            let candidate = color_capture(hint, scenario, "truecolor");
            for (y, (a, b)) in candidate.iter().zip(&plain).enumerate() {
                for (x, (a, b)) in a.iter().zip(b).enumerate() {
                    // ConPTY may merge invisible blank foregrounds into adjacent SGR runs.
                    // Exact source-cell styles are asserted against Ratatui buffers separately.
                    assert_eq!(
                        a.symbol, b.symbol,
                        "{scenario}, profile {hint}, glyph {x},{y}"
                    );
                    assert_eq!(a.bg, b.bg, "{scenario}, profile {hint}, background {x},{y}");
                    if !a.symbol.trim().is_empty() {
                        assert_eq!(a.fg, b.fg, "{scenario}, profile {hint}, foreground {x},{y}");
                    }
                }
            }
        }
        let star: Vec<_> = plain.iter().flatten().filter(|c| c.symbol == "✦").collect();
        assert_eq!(star.len(), if scenario == "welcome" { 2 } else { 1 });
        assert_eq!(star[0].fg, CapturedColor::Rgb(192, 229, 140));
        assert_eq!(star[0].bg, CapturedColor::Rgb(0, 0, 0));
        let footer = &plain[39];
        let auto = footer.iter().position(|c| c.symbol == "a").unwrap();
        assert_eq!(footer[auto].fg, CapturedColor::Rgb(245, 194, 231));
        assert!(
            plain
                .iter()
                .flatten()
                .any(|c| c.bg == CapturedColor::Rgb(0, 0, 0) || c.bg == CapturedColor::Indexed(0))
        );
        if scenario == "conversation" {
            assert!(
                plain
                    .iter()
                    .flatten()
                    .any(|c| c.bg == CapturedColor::Rgb(20, 20, 20))
            );
            assert!(
                plain
                    .iter()
                    .flatten()
                    .any(|c| c.symbol == "R" && c.fg == CapturedColor::Rgb(249, 226, 175))
            );
        }
    }
}
#[test]
fn conpty_color_depth_overrides_disabled_color_and_terminal_theme_are_respected() {
    let ansi = color_capture("ansi", "welcome", "ansi");
    let rgb = color_capture("force", "welcome", "truecolor");
    assert_ne!(ansi, rgb);
    for p in ["no-color", "dumb", "never-force"] {
        let cells = color_capture(p, "welcome", "none");
        assert!(
            cells
                .iter()
                .flatten()
                .all(|c| !matches!(c.fg, CapturedColor::Rgb(..))
                    && !matches!(c.bg, CapturedColor::Rgb(..))),
            "disabled {p} emitted RGB"
        );
    }
    for p in ["always-no-color", "always-dumb"] {
        let other = color_capture(p, "welcome", "truecolor");
        for (y, (a, b)) in other.iter().zip(&rgb).enumerate() {
            for (x, (a, b)) in a.iter().zip(b).enumerate() {
                assert_eq!(a.symbol, b.symbol, "profile {p} glyph {x},{y}");
                assert_eq!(a.bg, b.bg, "profile {p} background {x},{y}");
                if !a.symbol.trim().is_empty() {
                    assert_eq!(a.fg, b.fg, "profile {p} foreground {x},{y}");
                }
            }
        }
    }
    let terminal = color_capture("terminal", "welcome", "terminal");
    assert_ne!(terminal, rgb);
}

#[test]
fn conpty_midline_typing_mouse_paste_and_newline_keep_visible_cursor_at_editor_position() {
    let child = ConptyChild::spawn("welcome");
    child.wait_trace("ready for editing", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("auto mode on")
    });
    child.send(b"something");
    child.wait_trace("draft entered", |t| {
        t.lines().any(|l| l.contains("\"editor_cursor_byte\":9"))
            && terminal_state(&child.output_snapshot(), 120, 40).cursor == (15, 36)
    });
    child.send(b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D");
    child.wait_trace("cursor inside draft", |t| {
        t.lines().any(|l| l.contains("\"editor_cursor_byte\":4"))
            && terminal_state(&child.output_snapshot(), 120, 40).cursor == (10, 36)
    });
    let mut cursor = 4;
    for ch in b"text" {
        let before = terminal_state(&child.output_snapshot(), 120, 40);
        child.send(&[*ch]);
        cursor += 1;
        let expected = (6 + cursor, 36);
        child.wait_trace("middle insertion frame", |t| {
            let state = terminal_state(&child.output_snapshot(), 120, 40);
            state.cursor == expected
                && state.visible
                && t.lines()
                    .any(|l| l.contains(&format!("\"editor_cursor_byte\":{cursor}")))
        });
        let state = terminal_state(&child.output_snapshot(), 120, 40);
        assert!(
            state.reveals[before.reveals.len()..]
                .iter()
                .all(|p| *p == before.cursor || *p == expected),
            "cursor flashed away from edit position: {:?}",
            state.reveals
        );
    }
    assert!(terminal_screen(&child.output_snapshot(), 120, 40).contains("sometextthing"));
    child.send(b"\x08");
    child.wait_trace("backspace in middle", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("sometexthing")
            && terminal_state(&child.output_snapshot(), 120, 40).cursor == (13, 36)
    });
    child.send(b"\x1b[3~");
    child.wait_trace("delete in middle", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("sometexhing")
            && terminal_state(&child.output_snapshot(), 120, 40).cursor == (13, 36)
    });
    child.send(b"\x1b[<0;11;37M\x1b[<0;11;37m");
    child.wait_trace("mouse moves editor cursor", |_| {
        terminal_state(&child.output_snapshot(), 120, 40).cursor == (10, 36)
    });
    child.send("\x1b[200~λ界\x1b[201~".as_bytes());
    child.wait_trace("Unicode paste at mouse position", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("someλ界texhing")
            && terminal_state(&child.output_snapshot(), 120, 40).cursor == (13, 36)
    });
    child.send(b"\x1b[13;2u");
    child.wait_trace("newline inside draft", |t| {
        let state = terminal_state(&child.output_snapshot(), 120, 40);
        state.visible
            && state.cursor == (6, 36)
            && t.lines()
                .any(|l| l.contains("\"input_lines\":2") && l.contains("\"editor_cursor_byte\":10"))
    });
    fs::write(
        child.trace.with_extension("cursor.ansi"),
        child.output_snapshot(),
    )
    .unwrap();
    fs::write(
        child.trace.with_extension("cursor.jsonl"),
        child.trace_text(),
    )
    .unwrap();
    child.send(b"\x03\x03\x03");
    child.wait_trace("cursor test exit", |t| t.contains("\"quit\":true"));
    child.finish();
}

#[test]
fn conpty_keyboard_and_mouse_selection_emit_source_tinted_backgrounds_without_recoloring_text() {
    let text = "plain @file.rs /plan $websearch https://example.com";
    for profile in ["plain", "hint", "ansi"] {
        let child = ConptyChild::spawn_colors("welcome", profile);
        child.wait_trace("ready for selection", |_| {
            terminal_screen(&child.output_snapshot(), 120, 40).contains("auto mode on")
        });
        child.send(text.as_bytes());
        child.wait_trace("colored draft drawn", |_| {
            let bytes = child.output_snapshot();
            let state = terminal_state(&bytes, 120, 40);
            terminal_screen(&bytes, 120, 40).contains(text)
                && state.visible
                && state.cursor == (6 + text.len(), 36)
        });
        let before_bytes = child.output_snapshot();
        fs::write(
            child.trace.with_extension("selection-before.ansi"),
            &before_bytes,
        )
        .unwrap();
        let before = styled_terminal_screen(&before_bytes, 120, 40);
        child.send(b"\x1b[1;2H");
        let expected = if profile == "ansi" {
            CapturedColor::Indexed(237)
        } else {
            CapturedColor::Rgb(70, 70, 70)
        };
        child.wait_trace("keyboard selection drawn", |_| {
            let bytes = child.output_snapshot();
            let state = terminal_state(&bytes, 120, 40);
            state.visible && state.cursor == (6, 36) && state.cells[36][6].bg == expected
        });
        for mouse in [false, true] {
            if mouse {
                child.send(b"\x1b[F");
                child.wait_trace("keyboard selection cleared", |_| {
                    styled_terminal_screen(&child.output_snapshot(), 120, 40)[36][6].bg != expected
                });
                child.send(b"\x1b[<0;7;37M");
                child.send(format!("\x1b[<32;{};37M", 7 + text.len()).as_bytes());
                child.wait_trace("held mouse selection drawn", |_| {
                    let bytes = child.output_snapshot();
                    let state = terminal_state(&bytes, 120, 40);
                    state.visible && state.cells[36][6 + text.len() - 1].bg == expected
                });
            }
            let bytes = child.output_snapshot();
            let after = styled_terminal_screen(&bytes, 120, 40);
            fs::write(
                child
                    .trace
                    .with_extension(format!("{profile}-{mouse}.selection.ansi")),
                &bytes,
            )
            .unwrap();
            // ConPTY may coalesce invisible spaces into the preceding SGR run.
            // Check visible foregrounds here; exact source-cell colors (including
            // spaces) are checked in selection_tints.rs. Backgrounds are explicit.
            for offset in (0..text.len()).filter(|&i| !text.as_bytes()[i].is_ascii_whitespace()) {
                assert_eq!(
                    after[36][6 + offset].fg,
                    before[36][6 + offset].fg,
                    "{profile}, mouse={mouse}, byte {offset}"
                );
            }
            for (offset, rgb) in [
                (0, (70, 70, 70)),
                (5, (70, 70, 70)),
                (6, (75, 68, 53)),
                (15, (35, 56, 0)),
                (21, (41, 54, 75)),
            ] {
                let bg = if profile == "ansi" {
                    CapturedColor::Indexed(237)
                } else {
                    CapturedColor::Rgb(rgb.0, rgb.1, rgb.2)
                };
                assert_eq!(
                    after[36][6 + offset].bg,
                    bg,
                    "{profile}, mouse={mouse}, byte {offset}"
                );
            }
            assert_eq!(after[36][5].bg, before[36][5].bg, "prompt gutter shaded");
            assert_eq!(
                after[36][6 + text.len()].bg,
                before[36][6 + text.len()].bg,
                "trailing padding shaded"
            );
        }
        child.send(b"\x1b");
        child.wait_trace("mouse selection dismissed", |_| {
            styled_terminal_screen(&child.output_snapshot(), 120, 40)[36][6].bg != expected
        });
        // Do not release the mouse onto the OS clipboard: this fixture checks paint only.
        fs::write(
            child.trace.with_extension("selection.jsonl"),
            child.trace_text(),
        )
        .unwrap();
        child.send(b"\x03\x03\x03");
        child.wait_trace("selection test normal exit", |t| {
            t.contains("\"quit\":true")
        });
        child.finish();
    }
}

#[test]
fn conpty_settings_keyboard_mouse_usage_and_restart_share_saved_style() {
    use polaris::{
        app::App,
        interaction::{HitAction, UiMap},
        settings::SettingsAction,
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let config = std::env::current_dir()
        .unwrap()
        .join("target/conpty-traces")
        .join(format!(
            "settings-restart-{}-{}",
            std::process::id(),
            TRACE_ID.fetch_add(1, Ordering::Relaxed)
        ));
    let child = ConptyChild::spawn_config("settings", "inherit", Some(&config));
    child.wait_trace("settings ready with Strip default", |t| {
        start_contains(t, "Settings", "auto") && t.contains("\"usage_style\":\"Strip\"")
    });
    // Category focus: Down selects Usage, Tab moves into its style rows.
    child.send(b"\x1b[B\x1b[9;1u\x1b[B\x1b[13;1u");
    child.wait_trace("keyboard saves Ticks", |t| {
        t.lines().any(|l| {
            l.contains("\"usage_style\":\"Ticks\"")
                && l.contains("\"preferences_status\":\"Saved\"")
        })
    });
    let mut app = App::default();
    app.load_scenario("usage-bars").unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();
    let r = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Settings(SettingsAction::Choose { row: 2, value: 0 }))
        .unwrap()
        .rect;
    child.send(
        format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            r.x + 1,
            r.y + 1,
            r.x + 1,
            r.y + 1
        )
        .as_bytes(),
    );
    child.wait_trace("mouse highlights Segments without applying", |t| {
        t.lines()
            .any(|l| l.contains("\"selected\":2") && l.contains("\"usage_style\":\"Ticks\""))
    });
    let before: serde_json::Value =
        serde_json::from_slice(&fs::read(config.join("settings.json")).unwrap()).unwrap();
    assert_eq!(before["usage_bar_style"], "ticks");
    child.send(b"\x1b[13;1u");
    child.wait_trace("Enter saves mouse-selected Segments", |t| {
        t.lines().any(|l| {
            l.contains("\"usage_style\":\"Segments\"")
                && l.contains("\"preferences_status\":\"Saved\"")
        })
    });
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(config.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["usage_bar_style"], "segments");
    child.send(b"\x1b");
    child.wait_trace("Esc closes Settings", |t| {
        t.lines()
            .any(|l| l.contains("\"code\":\"Esc\"") && l.contains("\"screen\":\"Home\""))
    });
    child.send(b"/usage\r");
    child.wait_trace("usage uses saved Segments", |t| {
        t.lines().any(|l| {
            l.contains("\"screen\":\"Usage\"") && l.contains("\"usage_style\":\"Segments\"")
        })
    });
    child.send(b"\x0f");
    child.wait_trace("usage details still work", |t| {
        t.contains("\"usage_details\":true")
    });
    let screen = terminal_screen(&child.output_snapshot(), 120, 40);
    assert!(!screen.contains("C Compare") && !screen.contains("1 Ticks"));
    child.send(b"\r\x03\x03");
    child.wait_trace("normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
    let child = ConptyChild::spawn_config("usage-demo", "inherit", Some(&config));
    child.wait_trace("restart loads saved Segments", |t| {
        start_contains(t, "Usage", "auto") && t.contains("\"usage_style\":\"Segments\"")
    });
    child.wait_trace("usage redraw", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("DEMO DATA")
    });
    let screen = terminal_screen(&child.output_snapshot(), 120, 40);
    assert!(screen.contains('▮'));
    assert!(!screen.contains("C Compare"));
    child.send(b"\r\x03\x03");
    child.wait_trace("restart normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
}

#[test]
fn conpty_agents_keyboard_and_mouse_preserve_main_draft_and_route_focus() {
    use polaris::{
        agents::AgentAction,
        app::App,
        interaction::{HitAction, UiMap},
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let child = ConptyChild::spawn("agents-many");
    let latest = |t: &str, view: &str, focus: bool| {
        t.lines().last().is_some_and(|l| {
            l.contains(&format!("\"viewed_agent\":{view}"))
                && l.contains(&format!("\"agent_focus\":{focus}"))
        })
    };
    child.wait_trace("agent roster ready", |t| {
        t.contains("\"agent_count\":9")
            && terminal_screen(&child.output_snapshot(), 120, 40).contains("Agents [demo]")
    });
    child.send(b"main draft\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D\x1b[D");
    child.wait_trace("cursor inside Main draft", |t| {
        t.lines()
            .last()
            .is_some_and(|l| l.contains("\"editor_cursor_byte\":4"))
    });
    child.send(b"\x1bOR\x1b[B\x1b[13;1u");
    child.wait_trace("Enter views first agent and retains draft", |t| {
        latest(t, "1", false)
            && t.lines().last().is_some_and(|l| {
                l.contains("\"input_chars\":10") && l.contains("\"editor_cursor_byte\":4")
            })
    });
    child.wait_trace("agent conversation painted", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("prompt to Main")
    });
    child.send(b"X\x0f");
    child.wait_trace(
        "editing remains in Main composer while agent tools expand",
        |t| {
            latest(t, "1", false)
                && t.lines().last().is_some_and(|l| {
                    l.contains("\"editor_cursor_byte\":5")
                        && l.contains("\"expanded_tool_count\":1")
                })
        },
    );
    child.send(b"\x1bOQ");
    child.wait_trace("status overlays agent conversation", |t| {
        t.lines()
            .last()
            .is_some_and(|l| l.contains("\"screen\":\"Status\""))
    });
    child.send(b"\x1b");
    child.wait_trace("Escape returns to agent", |t| {
        latest(t, "1", false)
            && t.lines()
                .last()
                .is_some_and(|l| l.contains("\"screen\":\"Home\""))
    });
    let mut app = App::default();
    app.load_scenario("agents-many").unwrap();
    app.view_agent(Some(1));
    app.input.set("mainX draft");
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();
    let dock = map.agent_rect.unwrap();
    child.send(format!("\x1b[<65;{};{}M", dock.x + 1, dock.y + 1).as_bytes());
    child.wait_trace("wheel scrolls roster without changing agent", |t| {
        latest(t, "1", false)
            && terminal_screen(&child.output_snapshot(), 120, 40).contains("2-5 / 9 agents")
    });
    let r = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Agent(AgentAction::Open(None)))
        .unwrap()
        .rect;
    child.send(
        format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            r.x + 1,
            r.y + 1,
            r.x + 1,
            r.y + 1
        )
        .as_bytes(),
    );
    child.wait_trace("mouse returns to Main with draft", |t| {
        latest(t, "null", false)
            && terminal_screen(&child.output_snapshot(), 120, 40).contains("mainX draft")
    });
    fs::write(
        child.trace.with_extension("agents.ansi"),
        child.output_snapshot(),
    )
    .unwrap();
    fs::write(
        child.trace.with_extension("agents.jsonl"),
        child.trace_text(),
    )
    .unwrap();
    child.send(b"\x03\x03\x03");
    child.wait_trace("normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
}

#[test]
fn conpty_stats_all_variants_inspection_details_mouse_and_close_during_response() {
    use polaris::{
        app::App,
        interaction::{HitAction, UiMap},
        stats::{ChartStyle, StatsAction, StatsRange, StatsTab},
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend};
    let child = ConptyChild::spawn("working");
    let last = |t: &str, field: &str| t.lines().last().is_some_and(|l| l.contains(field));
    child.wait_trace("working ready", |t| t.contains("\"working\":true"));
    child.send(b"/stats\r");
    child.wait_trace("Stats command while working", |t| {
        last(t, "\"screen\":\"Stats\"") && last(t, "\"stats_demo\":false")
    });
    child.send(b"dr\x1b[9;1u");
    child.wait_trace("daily Models demo", |t| {
        last(t, "\"stats_tab\":\"Models\"")
            && last(t, "\"stats_range\":\"Week\"")
            && last(t, "\"stats_demo\":true")
    });
    child.send(b"c\x0f[");
    child.wait_trace("Lines details and interval inspection", |t| {
        last(t, "\"stats_chart\":\"Lines\"") && last(t, "\"stats_details\":true")
    });
    child.wait_trace("inspector painted", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("tokens")
    });
    let mut app = App::default();
    app.load_scenario("stats-demo").unwrap();
    app.stats.range = StatsRange::Week;
    app.stats.tab = StatsTab::Models;
    app.stats.details = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();
    let r = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Stats(StatsAction::Style(ChartStyle::Trends)))
        .unwrap()
        .rect;
    child.send(
        format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            r.x + 1,
            r.y + 1,
            r.x + 1,
            r.y + 1
        )
        .as_bytes(),
    );
    child.wait_trace("mouse previews Trends without saving", |t| {
        last(t, "\"stats_chart\":\"Trends\"") && last(t, "\"stats_default\":\"Stacked\"")
    });
    child.send(b"\x1b[<65;50;20M\x1b[6~");
    child.wait_trace("body scrolls", |t| {
        last(t, "\"screen\":\"Stats\"") && !last(t, "\"scroll\":0,")
    });
    child.wait_trace("pinned badge remains", |_| {
        terminal_screen(&child.output_snapshot(), 120, 40).contains("DEMO DATA")
    });
    fs::write(
        child.trace.with_extension("stats.ansi"),
        child.output_snapshot(),
    )
    .unwrap();
    fs::write(
        child.trace.with_extension("stats.jsonl"),
        child.trace_text(),
    )
    .unwrap();
    child.send(b"\x1b");
    child.wait_trace("Escape restores Main without interrupt", |t| {
        last(t, "\"screen\":\"Home\"") && last(t, "\"working\":true")
    });
    child.send(b"\x03\x03\x03");
    child.wait_trace("normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
}
#[test]
fn conpty_statistics_settings_require_enter_and_restore_saved_chart_on_restart() {
    let config = std::env::current_dir()
        .unwrap()
        .join("target/conpty-traces")
        .join(format!(
            "stats-settings-{}-{}",
            std::process::id(),
            TRACE_ID.fetch_add(1, Ordering::Relaxed)
        ));
    let child = ConptyChild::spawn_config("settings", "inherit", Some(&config));
    let last = |t: &str, field: &str| t.lines().last().is_some_and(|l| l.contains(field));
    child.wait_trace("Settings ready", |t| start_contains(t, "Settings", "auto"));
    child.send(b"\x1b[B\x1b[B\x1b[9;1u\x1b[B");
    child.wait_trace("Lines highlighted, default unchanged", |t| {
        last(t, "\"settings_category\":2")
            && last(t, "\"selected\":1,")
            && last(t, "\"stats_default\":\"Stacked\"")
    });
    assert!(!config.join("settings.json").exists());
    child.send(b"\x1b[13;1u");
    child.wait_trace("Enter saves Lines", |t| {
        last(t, "\"stats_default\":\"Lines\"") && last(t, "\"preferences_status\":\"Saved\"")
    });
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(config.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["stats_chart_style"], "lines");
    child.send(b"\x1b\x03\x03");
    child.wait_trace("settings normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
    let child = ConptyChild::spawn_config("stats-demo", "inherit", Some(&config));
    child.wait_trace("restart uses saved default", |t| {
        start_contains(t, "Stats", "auto") && last(t, "\"stats_default\":\"Lines\"")
    });
    child.send(b"\x1b[9;1u");
    child.wait_trace("Models uses Lines", |t| {
        last(t, "\"stats_tab\":\"Models\"") && last(t, "\"stats_chart\":\"Lines\"")
    });
    child.send(b"\r\x03\x03");
    child.wait_trace("restart normal exit", |t| t.contains("\"quit\":true"));
    child.finish();
}

#[test]
fn conpty_calendar_transports_full_day_backgrounds_in_all_color_profiles() {
    use polaris::{
        app::App,
        interaction::{HitAction, UiMap},
        stats::StatsAction,
        theme::ThemeMode,
        ui,
    };
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    for profile in ["plain", "ansi", "terminal", "never-force"] {
        let mut app = App::default();
        app.load_scenario("stats-demo").unwrap();
        if profile == "ansi" {
            app.theme.rgb = false;
        }
        if profile == "terminal" {
            app.theme.mode = ThemeMode::Terminal;
        }
        if profile == "never-force" {
            app.theme.color = false;
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|f| map = ui::render_interactive(f, &app))
            .unwrap();
        let mut expected = vec![];
        for target in &map.targets {
            if matches!(target.action, HitAction::Stats(StatsAction::Inspect(_))) {
                for x in target.rect.x..target.rect.right() {
                    let cell = &terminal.backend().buffer()[(x, target.rect.y)];
                    let bg = match cell.bg {
                        Color::Rgb(r, g, b) => CapturedColor::Rgb(r, g, b),
                        Color::Indexed(i) => CapturedColor::Indexed(i.into()),
                        Color::Reset => CapturedColor::Default,
                        other => panic!("unexpected tile color {other:?}"),
                    };
                    expected.push((
                        x as usize,
                        target.rect.y as usize,
                        bg,
                        cell.symbol().to_string(),
                    ));
                }
            }
        }
        assert!(!expected.is_empty());
        let child = ConptyChild::spawn_colors("stats-demo", profile);
        child.wait_trace("all calendar day cells painted", |_| {
            let cells = styled_terminal_screen(&child.output_snapshot(), 120, 40);
            expected.iter().all(|(x, y, bg, symbol)| {
                cells[*y][*x].bg == *bg && cells[*y][*x].symbol == *symbol
            })
        });
        fs::write(
            child
                .trace
                .with_extension(format!("calendar-{profile}.ansi")),
            child.output_snapshot(),
        )
        .unwrap();
        child.send(b"\r\x03\x03");
        child.wait_trace("normal calendar exit", |t| t.contains("\"quit\":true"));
        child.finish();
    }
}

#[test]
fn conpty_mouse_storms_and_capability_replies_do_not_repaint_unchanged_content() {
    for supported in [false, true] {
        let child = ConptyChild::spawn_colors("conversation", "plain");
        child.wait_trace("initial frame", |t| t.contains("\"event\":\"frame\""));
        // Let the asynchronous capability fallback finish before measuring idle output.
        thread::sleep(Duration::from_millis(350));
        let replies = child
            .trace_text()
            .lines()
            .filter(|l| l.contains("\"event\":\"terminal_reply\""))
            .count();
        let initial_frames = child
            .trace_text()
            .lines()
            .filter(|l| l.contains("\"event\":\"frame\""))
            .count();
        let start = child.output_snapshot().len();
        // Real terminals send complete mouse packets. Keep the burst rapid but
        // do not turn it into one oversized ConPTY input-parser write.
        for i in 0..1000 {
            child.send(format!("\x1b[<35;{};1M", 100 + i % 10).as_bytes());
            thread::sleep(Duration::from_millis(1));
        }
        child.send(if supported {
            b"\x1b[?2026;2$y"
        } else {
            b"\x1b[?2026;0$y"
        });
        child.wait_trace("burst processed", |t| {
            t.lines()
                .filter(|l| l.contains("\"event\":\"terminal_reply\""))
                .count()
                > replies
        });
        thread::sleep(Duration::from_millis(100));
        let after = child.output_snapshot();
        fs::write(
            child.trace.with_extension("burst.jsonl"),
            child.trace_text(),
        )
        .unwrap();
        fs::write(child.trace.with_extension("burst.ansi"), &after[start..]).unwrap();
        assert!(
            after[start..].is_empty(),
            "blank mouse moves wrote terminal bytes: {:?}",
            String::from_utf8_lossy(&after[start..])
        );
        assert_eq!(
            child
                .trace_text()
                .lines()
                .filter(|l| l.contains("\"event\":\"frame\""))
                .count(),
            initial_frames,
            "unchanged hover must not even construct another frame"
        );
        let frames = child
            .trace_text()
            .lines()
            .filter(|l| l.contains("\"event\":\"frame\""))
            .count();
        child.send(b"something\x1b[D\x1b[D\x1b[D\x1b[D\x1b[Dtext");
        child.wait_trace("middle insertion", |t| {
            t.lines().last().is_some_and(|l| {
                l.contains("\"input_chars\":13,") && l.contains("\"editor_cursor_byte\":8,")
            })
        });
        child.wait_trace("new frame", |t| {
            t.lines()
                .filter(|l| l.contains("\"event\":\"frame\""))
                .count()
                > frames
        });
        let state = terminal_screen(&child.output_snapshot(), 120, 40);
        assert!(state.contains("sometextthing"));
        assert!(
            !state.contains("2026;"),
            "terminal replies leaked into draft"
        );
        fs::write(
            child.trace.with_extension(if supported {
                "batched-sync.ansi"
            } else {
                "batched-fallback.ansi"
            }),
            child.output_snapshot(),
        )
        .unwrap();
        child.send(b"\x03\x03\x03");
        child.wait_trace("normal exit", |t| t.contains("\"quit\":true"));
        child.finish();
    }
}
