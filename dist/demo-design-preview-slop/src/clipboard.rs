//! Asynchronous system clipboard access for composer paste and copy actions.
use arboard::Clipboard;
use std::{
    io,
    sync::mpsc,
    thread::{self, JoinHandle},
};

pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_CLIPBOARD_IMAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPayload {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardOperationResult {
    Paste(ClipboardPayload),
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardResult {
    pub token: u64,
    pub result: Result<ClipboardOperationResult, String>,
}

/// Backend operations run only on the clipboard worker thread.
pub trait ClipboardBackend: Send + 'static {
    fn get_image(&mut self) -> Result<Option<ClipboardImage>, String>;
    fn get_text(&mut self) -> Result<Option<String>, String>;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

enum ClipboardRequest {
    Paste(u64),
    Copy(u64, String),
}

type BackendInit = Box<dyn FnOnce() -> Result<Box<dyn ClipboardBackend>, String> + Send>;

pub struct ClipboardService {
    requests: Option<mpsc::Sender<ClipboardRequest>>,
    completed: mpsc::Receiver<ClipboardResult>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardService {
    /// Start the system clipboard worker. Clipboard initialization happens there.
    pub fn new() -> io::Result<Self> {
        Self::spawn(Box::new(|| {
            Clipboard::new()
                .map(|clipboard| {
                    Box::new(ArboardBackend { clipboard }) as Box<dyn ClipboardBackend>
                })
                .map_err(|e| e.to_string())
        }))
    }

    /// Start a worker with a caller-provided backend, primarily for isolated tests.
    pub fn with_backend<B: ClipboardBackend>(backend: B) -> io::Result<Self> {
        Self::spawn(Box::new(move || Ok(Box::new(backend))))
    }

    fn spawn(init: BackendInit) -> io::Result<Self> {
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("clipboard-worker".into())
            .spawn(move || {
                let mut backend = init();
                while let Ok(request) = request_rx.recv() {
                    let (token, result) = match request {
                        ClipboardRequest::Paste(token) => (
                            token,
                            backend
                                .as_mut()
                                .map_err(|error| error.clone())
                                .and_then(|b| paste(b.as_mut())),
                        ),
                        ClipboardRequest::Copy(token, text) => (
                            token,
                            backend
                                .as_mut()
                                .map_err(|error| error.clone())
                                .and_then(|b| copy(b.as_mut(), &text)),
                        ),
                    };
                    if result_tx.send(ClipboardResult { token, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(request_tx),
            completed: result_rx,
            worker: Some(worker),
        })
    }

    pub fn send_paste(&self, token: u64) -> io::Result<()> {
        self.send(ClipboardRequest::Paste(token))
    }
    pub fn send_copy(&self, token: u64, text: String) -> io::Result<()> {
        self.send(ClipboardRequest::Copy(token, text))
    }
    fn send(&self, request: ClipboardRequest) -> io::Result<()> {
        self.requests
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "clipboard worker stopped"))?
            .send(request)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "clipboard worker stopped"))
    }
    pub fn try_recv(&self) -> Option<ClipboardResult> {
        self.completed.try_recv().ok()
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.requests.take();
        self.worker.take();
    }
}

fn paste(backend: &mut dyn ClipboardBackend) -> Result<ClipboardOperationResult, String> {
    match backend.get_image() {
        Ok(Some(image)) => {
            validate_image(&image)?;
            Ok(ClipboardOperationResult::Paste(ClipboardPayload::Image {
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            }))
        }
        Ok(None) => read_text(backend),
        Err(image_error) => match read_text(backend) {
            Ok(result) => Ok(result),
            Err(text_error) => Err(format!(
                "clipboard image read failed: {image_error}; text read failed: {text_error}"
            )),
        },
    }
}

fn read_text(backend: &mut dyn ClipboardBackend) -> Result<ClipboardOperationResult, String> {
    let text = backend
        .get_text()?
        .ok_or_else(|| "clipboard has no text or image".to_string())?;
    validate_text(&text)?;
    Ok(ClipboardOperationResult::Paste(ClipboardPayload::Text(
        text,
    )))
}

fn copy(
    backend: &mut dyn ClipboardBackend,
    text: &str,
) -> Result<ClipboardOperationResult, String> {
    validate_text(text)?;
    backend.set_text(text)?;
    Ok(ClipboardOperationResult::Copy)
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
        Err(format!(
            "clipboard text exceeds {} byte limit",
            MAX_CLIPBOARD_TEXT_BYTES
        ))
    } else {
        Ok(())
    }
}

fn validate_image(image: &ClipboardImage) -> Result<(), String> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "clipboard image dimensions overflow".to_string())?;
    if bytes > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err(format!(
            "clipboard image exceeds {} byte limit",
            MAX_CLIPBOARD_IMAGE_BYTES
        ));
    }
    if bytes != image.rgba.len() {
        return Err("clipboard image RGBA byte length does not match its dimensions".to_string());
    }
    Ok(())
}

struct ArboardBackend {
    clipboard: Clipboard,
}
impl ClipboardBackend for ArboardBackend {
    fn get_image(&mut self) -> Result<Option<ClipboardImage>, String> {
        match self.clipboard.get_image() {
            Ok(image) => Ok(Some(ClipboardImage {
                width: image.width,
                height: image.height,
                rgba: image.bytes.into_owned(),
            })),
            Err(error) => Err(error.to_string()),
        }
    }
    fn get_text(&mut self) -> Result<Option<String>, String> {
        self.clipboard
            .get_text()
            .map(Some)
            .map_err(|e| e.to_string())
    }
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.clipboard
            .set_text(text.to_owned())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    #[derive(Clone)]
    struct Fake {
        image: Option<ClipboardImage>,
        text: Option<String>,
        writes: Arc<Mutex<Vec<String>>>,
    }
    impl ClipboardBackend for Fake {
        fn get_image(&mut self) -> Result<Option<ClipboardImage>, String> {
            Ok(self.image.clone())
        }
        fn get_text(&mut self) -> Result<Option<String>, String> {
            Ok(self.text.clone())
        }
        fn set_text(&mut self, text: &str) -> Result<(), String> {
            self.writes.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }
    fn result(service: &ClipboardService) -> ClipboardResult {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = service.try_recv() {
                return result;
            }
            assert!(Instant::now() < until, "clipboard worker timed out");
            thread::yield_now();
        }
    }
    fn fake(
        image: Option<ClipboardImage>,
        text: Option<String>,
    ) -> (ClipboardService, Arc<Mutex<Vec<String>>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let service = ClipboardService::with_backend(Fake {
            image,
            text,
            writes: writes.clone(),
        })
        .unwrap();
        (service, writes)
    }
    #[test]
    fn paste_prefers_valid_image_then_text_and_preserves_token() {
        let image = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        };
        let (service, _) = fake(Some(image.clone()), Some("text".into()));
        service.send_paste(41).unwrap();
        assert_eq!(
            result(&service),
            ClipboardResult {
                token: 41,
                result: Ok(ClipboardOperationResult::Paste(ClipboardPayload::Image {
                    width: 1,
                    height: 1,
                    rgba: image.rgba
                }))
            }
        );
        let (service, _) = fake(None, Some("text".into()));
        service.send_paste(42).unwrap();
        assert_eq!(
            result(&service),
            ClipboardResult {
                token: 42,
                result: Ok(ClipboardOperationResult::Paste(ClipboardPayload::Text(
                    "text".into()
                )))
            }
        );
    }
    #[test]
    fn copy_writes_only_after_request_and_rejects_oversized_text() {
        let (service, writes) = fake(None, None);
        assert!(writes.lock().unwrap().is_empty());
        service.send_copy(9, "copied".into()).unwrap();
        assert_eq!(result(&service).result, Ok(ClipboardOperationResult::Copy));
        assert_eq!(*writes.lock().unwrap(), ["copied"]);
        service
            .send_copy(10, "x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1))
            .unwrap();
        assert!(result(&service).result.unwrap_err().contains("byte limit"));
        assert_eq!(*writes.lock().unwrap(), ["copied"]);
    }
    #[test]
    fn image_size_and_shape_are_validated_without_overflow() {
        assert!(
            validate_image(&ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![0; 4]
            })
            .is_ok()
        );
        assert!(
            validate_image(&ClipboardImage {
                width: usize::MAX,
                height: 2,
                rgba: vec![]
            })
            .is_err()
        );
        assert!(
            validate_image(&ClipboardImage {
                width: 2,
                height: 2,
                rgba: vec![0; 4]
            })
            .is_err()
        );
        assert!(
            validate_image(&ClipboardImage {
                width: MAX_CLIPBOARD_IMAGE_BYTES / 4 + 1,
                height: 1,
                rgba: vec![]
            })
            .is_err()
        );
    }
}
