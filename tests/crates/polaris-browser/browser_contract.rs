//! Accessibility outlines, perceptual screenshot comparison and the verified artifact session.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Mutex;

use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use polaris_browser::{
    BrowserError, BrowserSession, GuestArtifact, GuestBrowser, OutlineBudget, SandboxFiles,
    a11y_outline, visual_diff,
};
use polaris_sdk::ids::Cid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PAGE: &str = r#"{
  "role": "WebArea", "name": "Checkout",
  "children": [
    {"role": "generic", "children": [
      {"role": "heading", "name": "Your   cart", "level": 1},
      {"role": "generic", "children": [
        {"role": "textbox", "name": "Email", "value": "a@b.c", "focused": true},
        {"role": "checkbox", "name": "Save details", "checked": true},
        {"role": "button", "name": "Pay \"now\"", "disabled": true},
        {"role": "StaticText", "name": ""}
      ]}
    ]},
    {"role": "link", "name": "Back to shop"}
  ]
}"#;

#[test]
fn outlines_fold_structure_and_number_actionable_nodes() -> TestResult {
    let outline = a11y_outline(PAGE.as_bytes(), OutlineBudget::default())?;
    assert_eq!(
        outline.text,
        "- WebArea \"Checkout\"\n  - heading \"Your cart\" [level=1]\n  - textbox \"Email\" [value=\"a@b.c\"] [focused] [ref=e1]\n  - checkbox \"Save details\" [checked=true] [ref=e2]\n  - button \"Pay \\\"now\\\"\" [disabled] [ref=e3]\n  - link \"Back to shop\" [ref=e4]\n"
    );
    assert_eq!(outline.source_nodes, 9);
    assert!(!outline.truncated);
    let refs: Vec<(&str, &str)> = outline
        .targets
        .iter()
        .map(|target| (target.reference.as_str(), target.role.as_str()))
        .collect();
    assert_eq!(
        refs,
        [
            ("e1", "textbox"),
            ("e2", "checkbox"),
            ("e3", "button"),
            ("e4", "link")
        ]
    );
    Ok(())
}

#[test]
fn outlines_respect_the_budget_and_reject_bad_input() -> TestResult {
    let tight = a11y_outline(
        PAGE.as_bytes(),
        OutlineBudget {
            max_bytes: 4096,
            max_lines: 2,
        },
    )?;
    assert_eq!(tight.text.lines().count(), 2);
    assert!(tight.truncated);
    assert!(tight.targets.is_empty(), "only emitted nodes get refs");
    assert_eq!(
        a11y_outline(b"not json", OutlineBudget::default()),
        Err(BrowserError::Malformed("accessibility tree"))
    );
    Ok(())
}

fn png(image: GrayImage) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image).write_to(&mut bytes, ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

fn gradient() -> GrayImage {
    GrayImage::from_fn(128, 96, |x, y| Luma([((x * 2 + y) % 256) as u8]))
}

#[test]
fn perceptual_hashes_tell_same_pages_from_different_ones() -> TestResult {
    let base = png(gradient())?;
    let same = visual_diff(&base, &base)?;
    assert_eq!(same.distance, 0);
    assert!(same.similar);
    assert_eq!(same.changed_cells, Some(Vec::new()));

    let brighter = png(GrayImage::from_fn(128, 96, |x, y| {
        Luma([((x * 2 + y) % 256).saturating_add(6).min(255) as u8])
    }))?;
    assert!(
        visual_diff(&base, &brighter)?.similar,
        "a slight tone shift is the same page"
    );

    let checker = png(GrayImage::from_fn(128, 96, |x, y| {
        Luma([if (x / 16 + y / 16) % 2 == 0 { 0 } else { 255 }])
    }))?;
    let different = visual_diff(&base, &checker)?;
    assert!(!different.similar, "distance {}", different.distance);
    Ok(())
}

#[test]
fn a_changed_region_is_located_on_the_grid() -> TestResult {
    let mut changed = gradient();
    for y in 0..12 {
        for x in 0..16 {
            changed.put_pixel(x, y, Luma([255]));
        }
    }
    let diff = visual_diff(&png(gradient())?, &png(changed)?)?;
    assert_eq!(diff.changed_cells, Some(vec![(0, 0)]));
    let smaller = png(GrayImage::from_fn(64, 48, |x, y| {
        Luma([((x + y) % 256) as u8])
    }))?;
    assert_eq!(
        visual_diff(&png(gradient())?, &smaller)?.changed_cells,
        None
    );
    assert!(visual_diff(b"not an image", &smaller).is_err());
    Ok(())
}

fn cid(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

struct FakeGuest {
    screenshot: GuestArtifact,
    tree: GuestArtifact,
    calls: Mutex<Vec<String>>,
}

impl GuestBrowser for FakeGuest {
    async fn screenshot(
        &self,
        url: &str,
        width: u32,
        height: u32,
    ) -> Result<GuestArtifact, BrowserError> {
        self.calls
            .lock()
            .map_err(|_| BrowserError::Guest("poisoned".to_owned()))?
            .push(format!("{url} {width}x{height}"));
        Ok(self.screenshot.clone())
    }
    async fn a11y_tree(&self) -> Result<GuestArtifact, BrowserError> {
        Ok(self.tree.clone())
    }
}

struct FakeFiles(BTreeMap<String, Vec<u8>>);

impl SandboxFiles for FakeFiles {
    async fn download(&self, path: &str, limit: usize) -> Result<Vec<u8>, BrowserError> {
        let bytes = self
            .0
            .get(path)
            .cloned()
            .ok_or_else(|| BrowserError::Guest("missing file".to_owned()))?;
        if bytes.len() > limit {
            return Err(BrowserError::TooLarge {
                what: "download",
                limit,
            });
        }
        Ok(bytes)
    }
}

#[tokio::test]
async fn the_session_accepts_only_verified_sandbox_artifacts() -> TestResult {
    let shot = png(gradient())?;
    let guest = FakeGuest {
        screenshot: GuestArtifact {
            path: "/sandbox/.polaris/shot.png".to_owned(),
            content: cid(&shot),
        },
        tree: GuestArtifact {
            path: "/sandbox/.polaris/tree.json".to_owned(),
            content: cid(PAGE.as_bytes()),
        },
        calls: Mutex::new(Vec::new()),
    };
    let files = FakeFiles(BTreeMap::from([
        ("/sandbox/.polaris/shot.png".to_owned(), shot.clone()),
        (
            "/sandbox/.polaris/tree.json".to_owned(),
            PAGE.as_bytes().to_vec(),
        ),
    ]));
    let session = BrowserSession::new(guest, files, OutlineBudget::default());
    let screenshot = session
        .screenshot("http://127.0.0.1:3000/", 128, 96)
        .await?;
    assert_eq!((screenshot.width, screenshot.height), (128, 96));
    assert_eq!(screenshot.content, cid(&shot));
    assert_eq!(session.outline().await?.targets.len(), 4);
    let diff = session
        .compare_with("http://127.0.0.1:3000/", &shot)
        .await?;
    assert_eq!(diff.distance, 0);
    assert_eq!(
        session.screenshot("javascript:alert(1)", 10, 10).await,
        Err(BrowserError::Rejected(
            "only http, https and file URLs are rendered"
        ))
    );

    let tampered = BrowserSession::new(
        FakeGuest {
            screenshot: GuestArtifact {
                path: "/sandbox/shot.png".to_owned(),
                content: cid(b"something else"),
            },
            tree: GuestArtifact {
                path: "/etc/passwd".to_owned(),
                content: cid(b""),
            },
            calls: Mutex::new(Vec::new()),
        },
        FakeFiles(BTreeMap::from([("/sandbox/shot.png".to_owned(), shot)])),
        OutlineBudget::default(),
    );
    assert_eq!(
        tampered.screenshot("https://example.test/", 64, 64).await,
        Err(BrowserError::DigestMismatch)
    );
    assert_eq!(
        tampered.outline().await,
        Err(BrowserError::Rejected(
            "artifact is outside the sandbox workspace"
        ))
    );
    Ok(())
}
