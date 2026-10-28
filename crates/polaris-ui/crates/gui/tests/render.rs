//! GUI Welcome, pixel half (descriptor: specs/descriptors/ui_gui_welcome.json,
//! `welcome_renders_without_panic_in_software_renderer`).
//!
//! Renders the real window plus a real aurora frame through Slint's software renderer. With
//! `POLARIS_RENDER_OUT=<file.ppm>` the frame is also written out for visual review.

use std::error::Error;
use std::rc::Rc;

use polaris_ui_ascii_aurora::{Aurora, AuroraConfig};
use polaris_ui_gui::backend::{BackendEvent, BackendLink, MessageKind, SessionSummary};
use polaris_ui_gui::{AppOptions, attach, build_window};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{Image, PhysicalSize, PlatformError, Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer};

type TestResult = Result<(), Box<dyn Error>>;

/// Models as the configuration would supply them (test fixture, not built into the GUI).
fn test_models() -> Vec<polaris_ui_gui::ModelOption> {
    [
        ("vendor/model-low", "model-low"),
        ("vendor/model-high", "model-high"),
        ("vendor/model-max", "model-max"),
    ]
    .into_iter()
    .map(|(id, label)| polaris_ui_gui::ModelOption {
        id: id.to_owned(),
        label: label.to_owned(),
    })
    .collect()
}

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const FONT: &[u8] = include_bytes!("../assets/fonts/jetbrains-mono.ttf");

struct SoftwarePlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

#[test]
fn welcome_renders_without_panic_in_software_renderer() -> TestResult {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(SoftwarePlatform {
        window: window.clone(),
    }))?;
    let options = AppOptions {
        version: String::from("0.0.0-test"),
        workspace: String::from("C:/work/polaris"),
        state_dir: String::from("C:/work/polaris/.polaris/state"),
        reduced_motion: true,
        models: test_models(),
        default_model: 1,
    };
    let ui = build_window(&options)?;
    let (request_tx, _request_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let wiring = attach(
        &ui,
        BackendLink {
            requests: request_tx,
            events: event_rx,
        },
        &options,
    )?;
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    slint::ComponentHandle::show(&ui)?;

    // A real aurora frame for the atmosphere area (window minus 68px sidebar and 56px topbar).
    let mut aurora = Aurora::new(AuroraConfig::default(), FONT)?;
    let size = aurora.resize(WIDTH - 68, HEIGHT - 56)?;
    let mut frame = SharedPixelBuffer::<Rgba8Pixel>::new(size.width_px, size.height_px);
    aurora.render_rgba(12.0, frame.make_mut_bytes())?;
    ui.set_aurora_frame(Image::from_rgba8(frame));

    let stride = usize::try_from(WIDTH)?;
    let pixels = draw(&window, stride)?.ok_or("the window must have something to draw")?;
    let first = pixels.first().copied().ok_or("empty buffer")?;
    let distinct = pixels.iter().filter(|p| **p != first).count();
    assert!(
        distinct > 10_000,
        "only {distinct} pixels differ from the first one"
    );

    // With POLARIS_RENDER_OUT=<dir>, also write the welcome state and the open menus as PPM files.
    if let Some(dir) = std::env::var_os("POLARIS_RENDER_OUT") {
        let dir = std::path::PathBuf::from(dir);
        write_ppm(&dir.join("welcome.ppm"), &pixels)?;
        ui.set_popover_open(true);
        ui.set_context_open(true);
        settle();
        let open = draw(&window, stride)?.ok_or("menus did not redraw")?;
        write_ppm(&dir.join("welcome-menus.ppm"), &open)?;
        ui.set_popover_open(false);
        ui.set_context_open(false);
        ui.set_about_open(true);
        settle();
        let about = draw(&window, stride)?.ok_or("about did not redraw")?;
        write_ppm(&dir.join("welcome-about.ppm"), &about)?;

        // Session screen with a committed message and a sidebar list.
        ui.set_about_open(false);
        ui.set_draft("draft for the next message".into());
        ui.invoke_draft_edited("draft for the next message".into());
        ui.invoke_show_sessions();
        event_tx.send(BackendEvent::Opened {
            id: String::from("s1"),
            title: String::from("Aurora planning"),
            workspace: std::path::PathBuf::from("C:/work/polaris"),
            branch: Some(String::from("main")),
        })?;
        event_tx.send(BackendEvent::Message {
            kind: MessageKind::UserInput,
            text: String::from(
                "Plan tomorrow: port the welcome aurora, then wire the session view.",
            ),
        })?;
        event_tx.send(BackendEvent::Message {
            kind: MessageKind::Directive,
            text: String::from("$websearch(on)"),
        })?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let now = i64::try_from(now)?;
        event_tx.send(BackendEvent::Sessions(vec![
            SessionSummary {
                id: String::from("s1"),
                title: String::from("Aurora planning"),
                workspace: std::path::PathBuf::from("C:/work/polaris"),
                branch: Some(String::from("main")),
                updated_ms: now,
            },
            SessionSummary {
                id: String::from("s0"),
                title: String::new(),
                workspace: std::path::PathBuf::from("C:/work/polaris"),
                branch: None,
                updated_ms: now - 3 * 3_600_000,
            },
            SessionSummary {
                id: String::from("s9"),
                title: String::from("Landing page copy"),
                workspace: std::path::PathBuf::from("C:/work/polaris-landing"),
                branch: None,
                updated_ms: now - 2 * 86_400_000,
            },
        ]))?;
        wiring.pump(&ui);
        settle();
        let session = draw(&window, stride)?.ok_or("session did not redraw")?;
        write_ppm(&dir.join("session.ppm"), &session)?;
    }
    Ok(())
}

/// Lets the 160 ms fade animations finish (they run on real time here).
fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(400));
    slint::platform::update_timers_and_animations();
}

fn draw(
    window: &MinimalSoftwareWindow,
    stride: usize,
) -> Result<Option<Vec<Rgb8Pixel>>, Box<dyn Error>> {
    let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; usize::try_from(WIDTH * HEIGHT)?];
    let drawn = window.draw_if_needed(|renderer| {
        let _dirty = renderer.render(&mut pixels, stride);
    });
    Ok(drawn.then_some(pixels))
}

fn write_ppm(path: &std::path::Path, pixels: &[Rgb8Pixel]) -> TestResult {
    let mut ppm = format!(
        "P6
{WIDTH} {HEIGHT}
255
"
    )
    .into_bytes();
    for p in pixels {
        ppm.extend_from_slice(&[p.r, p.g, p.b]);
    }
    std::fs::write(path, ppm)?;
    Ok(())
}
