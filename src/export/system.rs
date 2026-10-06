//! The OS side of printing and PDF export (§3.2.5): finding a
//! Chromium-based browser (Chrome, Edge, Chromium, Brave — or the one in
//! `MNEMONIC_BROWSER`) to turn the themed HTML into a PDF headlessly, and
//! opening files/folders with the system's default app. Callers: `export`,
//! `app::reading`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Environment variable naming the browser executable to use for PDF.
pub const BROWSER_ENV: &str = "MNEMONIC_BROWSER";
/// Longest a headless PDF render may take before it is killed.
const PDF_TIMEOUT: Duration = Duration::from_secs(60);

/// A Chromium-based browser able to print to PDF headlessly, if any.
pub fn find_browser() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os(BROWSER_ENV).map(PathBuf::from) {
        return custom.is_file().then_some(custom);
    }
    candidates().into_iter().find(|p| p.is_file())
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let apps = [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Chromium.app/Contents/MacOS/Chromium",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
            "Vivaldi.app/Contents/MacOS/Vivaldi",
        ];
        let roots = [Some(PathBuf::from("/Applications")), dirs::home_dir().map(|h| h.join("Applications"))];
        for root in roots.into_iter().flatten() {
            out.extend(apps.iter().map(|a| root.join(a)));
        }
    } else if cfg!(target_os = "windows") {
        let rel = [
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
            r"Chromium\Application\chrome.exe",
        ];
        for var in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(var).map(PathBuf::from) {
                out.extend(rel.iter().map(|r| root.join(r)));
            }
        }
    } else {
        let names = [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "microsoft-edge-stable",
            "brave-browser",
        ];
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                out.extend(names.iter().map(|n| dir.join(n)));
            }
        }
    }
    out
}

/// `file://` URL for an absolute path.
pub fn file_url(path: &Path) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    const PATH_SET: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}');
    let raw = path.to_string_lossy().replace('\\', "/");
    let encoded = utf8_percent_encode(&raw, PATH_SET).to_string();
    if encoded.starts_with('/') { format!("file://{encoded}") } else { format!("file:///{encoded}") }
}

/// Prints `html` (a file) to `pdf` with `browser` in headless mode,
/// keeping CSS backgrounds (the theme colours) and without the browser's
/// own header/footer. Tries the legacy headless mode first (fast where it
/// still exists), then the current one.
pub fn html_to_pdf(browser: &Path, html: &Path, pdf: &Path) -> Result<()> {
    let _ = std::fs::remove_file(pdf);
    let mut errors = Vec::new();
    for mode in ["--headless=old", "--headless"] {
        match print_once(browser, mode, html, pdf) {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("{mode}: {e:#}")),
        }
    }
    bail!("the browser could not print the PDF ({})", errors.join("; "))
}

/// One headless print attempt. Some browsers keep running after writing
/// the file, so this waits for a complete PDF rather than for the process
/// to exit, then stops the browser.
fn print_once(browser: &Path, mode: &str, html: &Path, pdf: &Path) -> Result<()> {
    let scratch = std::env::temp_dir().join(format!("mnemonic-pdf-{}-{}", std::process::id(), mode.len()));
    std::fs::create_dir_all(&scratch).with_context(|| format!("creating {}", scratch.display()))?;
    // stderr goes to a file: a pipe nobody reads can fill up and stall
    // the browser.
    let log_path = scratch.join("browser.log");
    let log = std::fs::File::create(&log_path).with_context(|| format!("creating {}", log_path.display()))?;
    let mut child = Command::new(browser)
        .arg(mode)
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-crash-reporter")
        // A fresh profile otherwise asks the macOS keychain for access,
        // a prompt that can't appear headless and stalls the print.
        .arg("--use-mock-keychain")
        .arg("--password-store=basic")
        .arg("--no-pdf-header-footer")
        .arg("--print-to-pdf-no-header")
        .arg("--run-all-compositor-stages-before-draw")
        .arg(format!("--user-data-dir={}", profile_dir(&scratch).display()))
        .arg(format!("--print-to-pdf={}", pdf.display()))
        .arg(file_url(html))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .with_context(|| format!("starting {}", browser.display()))?;

    let started = Instant::now();
    let mut last_size = None;
    let outcome = loop {
        std::thread::sleep(Duration::from_millis(200));
        let size = std::fs::metadata(pdf).map(|m| m.len()).ok().filter(|s| *s > 0);
        if let Some(s) = size
            && last_size == Some(s)
            && pdf_is_complete(pdf)
        {
            break Ok(());
        }
        last_size = size;
        if let Some(status) = child.try_wait().context("waiting for the browser")? {
            break if pdf_is_complete(pdf) { Ok(()) } else { Err(anyhow::anyhow!("exited ({status}) without a PDF")) };
        }
        if started.elapsed() > PDF_TIMEOUT {
            break Err(anyhow::anyhow!("no PDF after {}s", PDF_TIMEOUT.as_secs()));
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    if outcome.is_err() {
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let tail: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).rev().take(2).collect();
        log::warn!("export: {} {mode}: {}", browser.display(), tail.join(" | "));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    outcome
}

/// Browser profile for headless printing, kept in the cache folder: a
/// brand-new profile has to initialise first (slow, and on some systems it
/// never finishes headless), a warm one prints in about a second.
fn profile_dir(scratch: &Path) -> PathBuf {
    dirs::cache_dir()
        .map(|c| c.join("mnemonic").join("pdf-browser-profile"))
        .unwrap_or_else(|| scratch.join("profile"))
}

/// A PDF file that has been written to the end (`%%EOF` trailer).
fn pdf_is_complete(pdf: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(pdf) else {
        return false;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len < 8 || f.seek(SeekFrom::Start(len.saturating_sub(32))).is_err() {
        return false;
    }
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).is_ok() && String::from_utf8_lossy(&tail).contains("%%EOF")
}

/// Opens a file or folder with the system's default application.
pub fn open_path(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("explorer");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().with_context(|| format!("opening {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_are_percent_encoded() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(file_url(Path::new("/tmp/Catatan Saya#1.html")), "file:///tmp/Catatan%20Saya%231.html");
    }

    #[test]
    fn complete_pdfs_end_with_eof() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n...").unwrap();
        assert!(!pdf_is_complete(&pdf));
        std::fs::write(&pdf, b"%PDF-1.4\n...\n%%EOF\n").unwrap();
        assert!(pdf_is_complete(&pdf));
        assert!(!pdf_is_complete(&dir.path().join("missing.pdf")));
    }
}
