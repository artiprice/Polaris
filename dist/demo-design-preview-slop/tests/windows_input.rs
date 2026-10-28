#![cfg(windows)]

use std::{fs, io, mem, os::windows::io::AsRawHandle, ptr};
use winapi::um::{
    consoleapi::{GetConsoleMode, SetConsoleMode},
    handleapi::CloseHandle,
    processthreadsapi::{
        CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess,
    },
    synchapi::WaitForSingleObject,
    winbase::{CREATE_NEW_CONSOLE, STARTF_USESHOWWINDOW},
    wincon::{
        ENABLE_VIRTUAL_TERMINAL_INPUT, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD, SHIFT_PRESSED,
        WriteConsoleInputW,
    },
};

fn probe_log() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .with_extension("input-probe.log")
}

#[test]
fn native_records_encoded_controls_and_console_mode_restoration_are_preserved() {
    let exe = std::env::current_exe().unwrap();
    let exe_wide: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut command: Vec<u16> = format!(
        "\"{}\" --ignored --exact isolated_console_probe --nocapture",
        exe.display()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    // SAFETY: both Win32 structs are POD; cb is set as required. All strings
    // are NUL-terminated and stay live until CreateProcessW returns.
    let mut startup: STARTUPINFOW = unsafe { mem::zeroed() };
    startup.cb = mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESHOWWINDOW;
    startup.wShowWindow = 0; // SW_HIDE: the probe never opens a visible terminal.
    let mut process: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            exe_wide.as_ptr(),
            command.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            CREATE_NEW_CONSOLE,
            ptr::null_mut(),
            ptr::null(),
            &mut startup,
            &mut process,
        )
    };
    assert_ne!(created, 0, "{}", io::Error::last_os_error());
    // SAFETY: these handles belong to this child. Bound the wait and close both
    // handles on every path, terminating only this probe if it hangs.
    let (wait, exit) = unsafe {
        let wait = WaitForSingleObject(process.hProcess, 20_000);
        if wait != 0 {
            TerminateProcess(process.hProcess, 1);
            WaitForSingleObject(process.hProcess, 5_000);
        }
        let mut exit = 1;
        GetExitCodeProcess(process.hProcess, &mut exit);
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
        (wait, exit)
    };
    let evidence = fs::read_to_string(probe_log()).unwrap_or_default();
    assert_eq!(wait, 0, "hidden console probe timed out: {evidence}");
    assert_eq!(exit, 0, "hidden console probe failed: {evidence}");
    assert!(
        evidence.contains("SHIFT_ENTER_INSERTED_NEWLINE"),
        "{evidence}"
    );
    assert!(evidence.contains("PLAIN_ENTER_SUBMITTED"), "{evidence}");
    assert!(
        evidence.contains("ENCODED_CONSOLE_RECORD_SHIFT_ENTER_INSERTED_NEWLINE"),
        "{evidence}"
    );
    assert!(
        evidence.contains("ORIGINAL_INPUT_MODE_RESTORED"),
        "{evidence}"
    );
    assert!(
        evidence.contains("ORIGINAL_OUTPUT_MODE_RESTORED_NORMAL_ERROR_UNWIND"),
        "{evidence}"
    );
    println!("{evidence}");
}

// The parent test above always executes this in its own hidden Windows console.
// Running it directly inside a shared test runner could consume the user's input.
#[test]
#[ignore = "executed by the parent regression test in an isolated hidden console"]
fn isolated_console_probe() {
    let mut evidence = String::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
        use polaris::{
            app::App,
            input::{NativeConsoleInput, TerminalInput},
        };
        let console = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONIN$")
            .unwrap();
        let handle = console.as_raw_handle().cast();
        let mut original = 0;
        // SAFETY: console owns handle; original points to a live u32.
        assert_ne!(unsafe { GetConsoleMode(handle, &mut original) }, 0);
        let inherited = original & !ENABLE_VIRTUAL_TERMINAL_INPUT;
        assert_ne!(unsafe { SetConsoleMode(handle, inherited) }, 0);
        let guard = NativeConsoleInput::enter().unwrap();
        let mut actual = 0;
        assert_ne!(unsafe { GetConsoleMode(handle, &mut actual) }, 0);
        assert_eq!(actual, inherited | ENABLE_VIRTUAL_TERMINAL_INPUT);
        evidence.push_str(&format!(
            "input mode: inherited={inherited:#x}, terminal={actual:#x}\n"
        ));
        crossterm::terminal::enable_raw_mode().unwrap();
        // WriteConsoleInputW under VT input re-encodes native records and can
        // discard Shift+Enter modifiers. Probe native records with VT disabled,
        // then enable VT again for raw controls. ConPTY tests cover the actual
        // runtime mode and terminal bytes separately.
        assert_ne!(
            unsafe { SetConsoleMode(handle, actual & !ENABLE_VIRTUAL_TERMINAL_INPUT) },
            0
        );
        let mut input = TerminalInput::new().unwrap();
        let mut app = App::default();
        app.input.set("first paragraph");
        for (state, shifted) in [(SHIFT_PRESSED, true), (0, false)] {
            let mut record: INPUT_RECORD = unsafe { mem::zeroed() };
            record.EventType = KEY_EVENT;
            let mut key: KEY_EVENT_RECORD = unsafe { mem::zeroed() };
            key.bKeyDown = 1;
            key.wRepeatCount = 1;
            key.wVirtualKeyCode = 0x0D; // VK_RETURN
            key.wVirtualScanCode = 0x1C;
            key.dwControlKeyState = state;
            // SAFETY: KEY_EVENT is the active union tag, with valid u16 data.
            unsafe {
                *key.uChar.UnicodeChar_mut() = 13;
                *record.Event.KeyEvent_mut() = key;
            }
            let mut written = 0;
            assert_ne!(
                unsafe { WriteConsoleInputW(handle, &record, 1, &mut written) },
                0
            );
            assert_eq!(written, 1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let received = loop {
                assert!(
                    std::time::Instant::now() < deadline,
                    "missing console key event"
                );
                if input.poll(std::time::Duration::from_millis(50)).unwrap()
                    && let Event::Key(key) = input.read().unwrap()
                    && key.kind != KeyEventKind::Release
                {
                    break key;
                }
            };
            evidence.push_str(&format!("received {received:?}\n"));
            assert_eq!(received.code, KeyCode::Enter);
            assert_eq!(received.modifiers.contains(KeyModifiers::SHIFT), shifted);
            app.key(received);
            if shifted {
                assert_eq!(app.input.text, "first paragraph\n");
                assert!(!app.working);
                assert!(app.entries.is_empty());
                evidence.push_str("SHIFT_ENTER_INSERTED_NEWLINE\n");
                app.paste("second paragraph");
            } else {
                assert!(app.working);
                assert_eq!(
                    app.entries[0].lines,
                    ["first paragraph", "second paragraph"]
                );
                evidence.push_str("PLAIN_ENTER_SUBMITTED\n");
            }
        }
        assert_ne!(unsafe { SetConsoleMode(handle, actual) }, 0);
        // This probes the native record reader only. The separate ConPTY smoke
        // tests exercise real terminal bytes through the full OS transport.
        let mut decoder = polaris::input::KeyboardDecoder::default();
        let mut encoded_app = App::default();
        encoded_app.input.set("encoded first");
        let mut decoded_keys = Vec::new();
        for ch in "\x1b[13;2u".chars() {
            let mut record: INPUT_RECORD = unsafe { mem::zeroed() };
            record.EventType = KEY_EVENT;
            let mut key: KEY_EVENT_RECORD = unsafe { mem::zeroed() };
            key.bKeyDown = 1;
            key.wRepeatCount = 1;
            key.wVirtualKeyCode = 0; // ConPTY raw controls can have no virtual key.
            unsafe {
                *key.uChar.UnicodeChar_mut() = ch as u16;
                *record.Event.KeyEvent_mut() = key;
            }
            let mut written = 0;
            assert_ne!(
                unsafe { WriteConsoleInputW(handle, &record, 1, &mut written) },
                0
            );
            assert_eq!(written, 1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                assert!(
                    std::time::Instant::now() < deadline,
                    "missing CSI-u input character {ch:?}"
                );
                if input.poll(std::time::Duration::from_millis(50)).unwrap()
                    && let Event::Key(received) = input.read().unwrap()
                    && received.kind != KeyEventKind::Release
                {
                    evidence.push_str(&format!("CSI-u character: {received:?}\n"));
                    decoded_keys.extend(decoder.feed(received));
                    break;
                }
            }
        }
        assert_eq!(
            decoded_keys,
            [crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::SHIFT
            )]
        );
        for key in decoded_keys {
            encoded_app.key(key);
        }
        assert_eq!(encoded_app.input.text, "encoded first\n");
        assert!(!encoded_app.working);
        evidence.push_str("ENCODED_CONSOLE_RECORD_SHIFT_ENTER_INSERTED_NEWLINE\n");
        crossterm::terminal::disable_raw_mode().unwrap();
        drop(guard);
        assert_ne!(unsafe { GetConsoleMode(handle, &mut actual) }, 0);
        assert_eq!(actual, inherited);
        evidence.push_str("ORIGINAL_INPUT_MODE_RESTORED\n");
        use polaris::output::ConsoleOutputMode;
        use winapi::um::wincon::{ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING};
        let stdout = io::stdout();
        let output = stdout.as_raw_handle().cast();
        let mut output_original = 0;
        assert_ne!(unsafe { GetConsoleMode(output, &mut output_original) }, 0);
        for preset in [
            output_original & !ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            output_original | ENABLE_VIRTUAL_TERMINAL_PROCESSING | ENABLE_PROCESSED_OUTPUT,
        ] {
            assert_ne!(unsafe { SetConsoleMode(output, preset) }, 0);
            {
                let guard = ConsoleOutputMode::enter();
                assert!(guard.vt_available());
                let mut enabled = 0;
                assert_ne!(unsafe { GetConsoleMode(output, &mut enabled) }, 0);
                assert_eq!(
                    enabled,
                    preset | ENABLE_VIRTUAL_TERMINAL_PROCESSING | ENABLE_PROCESSED_OUTPUT
                );
            }
            let mut restored = 0;
            assert_ne!(unsafe { GetConsoleMode(output, &mut restored) }, 0);
            assert_eq!(restored, preset);
            fn fail_output_setup() -> io::Result<()> {
                let _guard = ConsoleOutputMode::enter();
                Err(io::Error::other("controlled setup failure"))
            }
            let setup_failure = fail_output_setup();
            assert!(setup_failure.is_err());
            assert_ne!(unsafe { GetConsoleMode(output, &mut restored) }, 0);
            assert_eq!(restored, preset);
            let panic = std::panic::catch_unwind(|| {
                let _guard = ConsoleOutputMode::enter();
                panic!("controlled output-mode unwind");
            });
            assert!(panic.is_err());
            assert_ne!(unsafe { GetConsoleMode(output, &mut restored) }, 0);
            assert_eq!(restored, preset);
        }
        assert_ne!(unsafe { SetConsoleMode(output, output_original) }, 0);
        evidence.push_str("ORIGINAL_OUTPUT_MODE_RESTORED_NORMAL_ERROR_UNWIND\n");
    }));
    fs::write(probe_log(), &evidence).unwrap();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
