use futures::channel::oneshot;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use windows_core::w;
use wry::WebViewExtWindows;

const MAX_WIDTH: f64 = 800.;

pub fn png(webview: &wry::WebView, rect: [f64; 4]) -> oneshot::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = oneshot::channel();
    let scale = crate::overlay::window_of(webview).map_or(1., |hwnd| unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd as _) }.max(96) as f64 / 96.);
    let tx = std::cell::Cell::new(Some(tx));
    let answer = std::rc::Rc::new(tx);
    let done = answer.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json: String| {
        let png = result.ok().and_then(|()| {
            let reply: serde_json::Value = serde_json::from_str(&json).ok()?;
            let shot = crate::attach::unbase64(reply["data"].as_str()?)?;
            cut(&shot, rect, scale)
        });
        if let Some(tx) = done.take() {
            let _ = tx.send(png);
        }
        Ok(())
    }));
    // SAFETY: on the main thread, as the rest of the app's WebView2 calls.
    let asked = unsafe { webview.webview().CallDevToolsProtocolMethod(w!("Page.captureScreenshot"), w!(r#"{"format":"png"}"#), &handler) };
    if asked.is_err()
        && let Some(tx) = answer.take()
    {
        let _ = tx.send(None);
    }
    rx
}

fn cut(shot: &[u8], [x, y, w, h]: [f64; 4], scale: f64) -> Option<Vec<u8>> {
    let image = image::load_from_memory_with_format(shot, image::ImageFormat::Png).ok()?;
    let px = |v: f64| (v * scale).round().max(0.) as u32;
    let (left, top) = (px(x).min(image.width()), px(y).min(image.height()));
    let (width, height) = (px(w).min(image.width() - left), px(h).min(image.height() - top));
    if width < 2 || height < 2 {
        return None;
    }
    let mut part = image.crop_imm(left, top, width, height);
    let widest = (MAX_WIDTH * scale) as u32;
    if width > widest {
        part = part.resize(widest, u32::MAX, image::imageops::FilterType::Triangle);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    part.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
