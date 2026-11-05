//! Saving an image from the notebook page: "Save Image As" in its menu on
//! Linux, "Download Image" on macOS. WebKit starts a download for either, and
//! the web view handled neither: on macOS nothing happened, and on Linux
//! WebKitGTK wrote the file, unasked, into the app's working folder under a
//! name made from the image's data. Now a Save dialog asks where it goes.
//!
//! Windows isn't here: WebView2 shows its own Save dialog.

/// A download the page started, waiting for the Save dialog's answer.
pub struct Ask {
    /// What the download fetches: for a figure, a `data:` or `blob:` URL.
    url: String,
    /// The file name WebKit suggested.
    suggested: String,
    /// macOS: WebKit's completion handler, which must be called once, even
    /// when the ask is dropped unanswered (WebKit raises otherwise).
    #[cfg(target_os = "macos")]
    done: Option<block2::RcBlock<dyn Fn(*mut objc2::runtime::AnyObject)>>,
}

#[cfg(target_os = "macos")]
impl Drop for Ask {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            done.call((std::ptr::null_mut(),));
        }
    }
}

impl Ask {
    /// The name the Save dialog offers.
    pub fn file_name(&self) -> String {
        file_name(&self.url, &self.suggested)
    }
}

/// WebKit's suggestion, unless it isn't a name: empty, "Unknown", or made
/// from the URL itself, as WebKitGTK does for a `data:` or `blob:` figure (the
/// end of its data). Those are "figure" with the image's extension. A link
/// that names its file (PlutoUI's DownloadButton) keeps that name.
fn file_name(url: &str, suggested: &str) -> String {
    let suggested = suggested.trim();
    let stem = suggested.rsplit_once('.').map_or(suggested, |(stem, _)| stem);
    let inline = url.starts_with("data:") || url.starts_with("blob:");
    let unnamed = stem.is_empty() || stem.eq_ignore_ascii_case("unknown") || (inline && url.contains(stem));
    if !unnamed {
        return suggested.to_string();
    }
    let from_type = url
        .strip_prefix("data:image/")
        .and_then(|rest| rest.split([';', ',']).next())
        .map(|subtype| match subtype {
            "jpeg" => "jpg",
            "svg+xml" => "svg",
            other => other,
        });
    let from_name = suggested.rsplit_once('.').map(|(_, ext)| ext);
    let ext = [from_type, from_name]
        .into_iter()
        .flatten()
        .find(|ext| !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("png");
    format!("figure.{}", ext.to_ascii_lowercase())
}

/// The user said "Replace" in the Save dialog, but WebKit won't write over a
/// file: it fails the download.
/// ponytail: the old file goes before the download has succeeded, so a failed
/// save loses it. Writing to a temporary name and renaming would keep it.
fn make_room(path: &std::path::Path) {
    if path.is_file()
        && let Err(e) = std::fs::remove_file(path)
    {
        eprintln!("Couldn't replace {}: {e}", path.display());
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use futures::channel::mpsc::UnboundedSender;

    use super::Ask;

    /// The download started again once the Save dialog answered, where it
    /// goes, and when it was started.
    static CHOSEN: Mutex<Option<(String, PathBuf, Instant)>> = Mutex::new(None);

    /// A restarted download begins at once. One that didn't (a `blob:` the page
    /// has let go of) mustn't send a later download of that URL there unasked.
    const RESTART: Duration = Duration::from_secs(10);

    /// wry's download handler. It must say at once where a download goes, so a
    /// new download is cancelled and asked about; `save` starts it again.
    pub fn started(asks: UnboundedSender<Ask>) -> impl FnMut(String, &mut PathBuf) -> bool + 'static {
        move |url, path| {
            let mut chosen = CHOSEN.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((asked, to, at)) = chosen.take()
                && asked == url
                && at.elapsed() < RESTART
            {
                *path = to;
                return true;
            }
            let suggested = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let _ = asks.unbounded_send(Ask { url, suggested });
            false
        }
    }

    pub fn save(webview: &wry::WebView, ask: Ask, to: Option<PathBuf>) {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        let Some(to) = to else { return };
        super::make_room(&to);
        *CHOSEN.lock().unwrap_or_else(|e| e.into_inner()) = Some((ask.url.clone(), to, Instant::now()));
        webview.webview().download_uri(&ask.url);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use block2::{Block, RcBlock};
    use futures::channel::mpsc::UnboundedSender;
    use objc2::ffi::{class_addMethod, object_getClass};
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{class, msg_send, sel};

    use super::Ask;
    use crate::dialogs::{ns_string, string_of};

    type MenuDownload = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject);
    type Decide = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut Block<dyn Fn(*mut AnyObject)>);
    type Failed = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject);

    thread_local! {
        static ASKS: RefCell<Option<UnboundedSender<Ask>>> = const { RefCell::new(None) };
    }

    /// The menu's "Download Image" made a download. With no delegate it goes
    /// nowhere; the navigation delegate takes it.
    unsafe extern "C-unwind" fn menu_did_create_download(this: *mut AnyObject, _: Sel, _: *mut AnyObject, download: *mut AnyObject) {
        unsafe {
            let _: () = msg_send![download, setDelegate: this];
        }
    }

    unsafe extern "C-unwind" fn decide_destination(
        _: *mut AnyObject,
        _: Sel,
        download: *mut AnyObject,
        _: *mut AnyObject,
        suggested: *mut AnyObject,
        done: *mut Block<dyn Fn(*mut AnyObject)>,
    ) {
        unsafe {
            let Some(done) = RcBlock::copy(done) else { return };
            let request: *mut AnyObject = msg_send![download, originalRequest];
            let url: *mut AnyObject = if request.is_null() { std::ptr::null_mut() } else { msg_send![request, URL] };
            let url = if url.is_null() { String::new() } else { string_of(msg_send![url, absoluteString]) };
            let ask = Ask { url, suggested: string_of(suggested), done: Some(done) };
            // Unsent, the ask is dropped here, which cancels the download.
            ASKS.with(|a| {
                if let Some(tx) = a.borrow().as_ref() {
                    let _ = tx.unbounded_send(ask);
                }
            });
        }
    }

    unsafe extern "C-unwind" fn download_failed(_: *mut AnyObject, _: Sel, _: *mut AnyObject, error: *mut AnyObject, _: *mut AnyObject) {
        unsafe {
            if error.is_null() {
                return;
            }
            // NSURLErrorCancelled: the user pressed Cancel.
            let code: isize = msg_send![error, code];
            if code == -999 {
                return;
            }
            eprintln!("Couldn't save the image: {}", string_of(msg_send![error, localizedDescription]));
        }
    }

    /// Let the page's image downloads ask where to go. The methods go on wry's
    /// navigation delegate's class, as in `webcontent::on_process_ended`.
    pub fn attach(webview: &wry::WebView, asks: UnboundedSender<Ask>) {
        use wry::WebViewExtMacOS;
        ASKS.with(|a| *a.borrow_mut() = Some(asks));
        let view = webview.webview();
        unsafe {
            let delegate: *mut AnyObject = msg_send![&*view, navigationDelegate];
            if delegate.is_null() {
                eprintln!("The notebook's web view has no navigation delegate; its images can't be saved");
                return;
            }
            let class = object_getClass(delegate) as *mut AnyClass;
            let methods: [(Sel, Imp, &std::ffi::CStr); 3] = [
                // WebKit's private navigation delegate method (macOS 11.3+) for the menu's downloads.
                (sel!(_webView:contextMenuDidCreateDownload:), std::mem::transmute::<MenuDownload, Imp>(menu_did_create_download), c"v@:@@"),
                (
                    sel!(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:),
                    std::mem::transmute::<Decide, Imp>(decide_destination),
                    c"v@:@@@@?",
                ),
                (sel!(download:didFailWithError:resumeData:), std::mem::transmute::<Failed, Imp>(download_failed), c"v@:@@@"),
            ];
            for (sel, imp, types) in methods {
                if !class_addMethod(class, sel, imp, types.as_ptr()).as_bool() {
                    eprintln!("wry's navigation delegate already has {}; saving images may not work", sel.name().to_string_lossy());
                }
            }
            // WebKit reads which methods a delegate has when it's set.
            let _: () = msg_send![&*view, setNavigationDelegate: delegate];
        }
    }

    pub fn save(_: &wry::WebView, mut ask: Ask, to: Option<PathBuf>) {
        // Dropping an unanswered ask cancels the download.
        let Some(to) = to else { return };
        let Some(done) = ask.done.take() else { return };
        super::make_room(&to);
        unsafe {
            let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: &*ns_string(&to.to_string_lossy())];
            done.call((url,));
        }
    }
}

pub use imp::*;

/// The folder the last image was saved in, where the next Save dialog opens.
/// Only for this run of the app.
static LAST_FOLDER: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Where the Save dialog opens: the last folder used, else Downloads, else home.
fn start_folder() -> std::path::PathBuf {
    let last = LAST_FOLDER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let home = wire::files::home();
    [last, Some(home.join("Downloads"))].into_iter().flatten().find(|dir| dir.is_dir()).unwrap_or(home)
}

impl crate::Workspace {
    /// Ask where the image `ask` goes, then save it there (or drop it).
    pub(crate) fn save_image(&mut self, ask: Ask, cx: &mut gpui::Context<Self>) {
        let picked = cx.prompt_for_new_path(&start_folder(), Some(&ask.file_name()));
        let webview = self.webview.clone();
        cx.spawn(async move |_, cx| {
            let to = picked.await.ok().and_then(Result::ok).flatten();
            if let Some(folder) = to.as_deref().and_then(std::path::Path::parent) {
                *LAST_FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(folder.to_path_buf());
            }
            webview.update(cx, |view, _| save(view.raw(), ask, to));
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn figures_are_named_figure() {
        assert_eq!(file_name("data:image/png;base64,iVBORw0KGgo/cR9NwAAAAASUVORK5CYII=", "cR9NwAAAAASUVORK5CYII=.png"), "figure.png");
        assert_eq!(file_name("data:image/jpeg;base64,/9j/4AAQ", "Unknown"), "figure.jpg");
        assert_eq!(file_name("data:image/svg+xml,%3Csvg", ""), "figure.svg");
        assert_eq!(file_name("blob:http://127.0.0.1:1234/8c1e", "8c1e"), "figure.png");
        assert_eq!(file_name("blob:null/8c1e", "8c1e.webp"), "figure.webp");
    }

    #[test]
    fn named_images_keep_their_name() {
        assert_eq!(file_name("http://127.0.0.1:1234/plot.png", "plot.png"), "plot.png");
        assert_eq!(file_name("http://127.0.0.1:1234/plot", ""), "figure.png");
        // PlutoUI's DownloadButton: a data: or blob: link with download="…".
        assert_eq!(file_name("data:text/csv;base64,YSxiCjEsMg==", "results.csv"), "results.csv");
        assert_eq!(file_name("blob:http://127.0.0.1:1234/8c1e", "plot.png"), "plot.png");
    }
}
