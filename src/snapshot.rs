//! A PNG of part of the notebook, for a box drawn with Point: WKWebView's own
//! snapshot of a rect, which draws what the page shows (plots included)
//! without screen-recording permission.

use std::cell::Cell;

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::overlay::CGRect;

/// Wider boxes are scaled down to this many points (twice as many pixels on
/// a Retina screen), which keeps the PNG well under the image cap.
const MAX_WIDTH: f64 = 800.;

/// NSBitmapImageFileTypePNG.
const PNG: usize = 4;

/// `rect` is x, y, width, height in the web view's points, from its top left.
/// The receiver gets None when WebKit couldn't draw it.
pub fn png(webview: &wry::WebView, rect: [f64; 4]) -> oneshot::Receiver<Option<Vec<u8>>> {
    use wry::WebViewExtMacOS;
    let (tx, rx) = oneshot::channel();
    let tx = Cell::new(Some(tx));
    let view = webview.webview();
    unsafe {
        let Some(config): Option<Retained<AnyObject>> = msg_send![class!(WKSnapshotConfiguration), new] else { return rx };
        let _: () = msg_send![&*config, setRect: CGRect::new(rect[0], rect[1], rect[2], rect[3])];
        if rect[2] > MAX_WIDTH {
            let width: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: MAX_WIDTH];
            let _: () = msg_send![&*config, setSnapshotWidth: &*width];
        }
        let handler = RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
            let png = if image.is_null() { None } else { encode(image) };
            if let Some(tx) = tx.take() {
                let _ = tx.send(png);
            }
        });
        let _: () = msg_send![&*view, takeSnapshotWithConfiguration: &*config, completionHandler: &*handler];
    }
    rx
}

/// An NSImage as PNG bytes.
unsafe fn encode(image: *mut AnyObject) -> Option<Vec<u8>> {
    unsafe {
        let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
        if tiff.is_null() {
            return None;
        }
        let bitmap: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        if bitmap.is_null() {
            return None;
        }
        let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        let data: *mut AnyObject = msg_send![bitmap, representationUsingType: PNG, properties: properties];
        if data.is_null() {
            return None;
        }
        let length: usize = msg_send![data, length];
        let bytes: *const u8 = msg_send![data, bytes];
        (!bytes.is_null() && length > 0).then(|| std::slice::from_raw_parts(bytes, length).to_vec())
    }
}
