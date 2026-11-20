//! A PNG of part of the notebook (macOS: `src/snapshot.rs`): WebKitGTK's own
//! snapshot of what the web view shows, cut to a rect. It draws the page
//! itself, plots and WebGL included, whatever is in front of the view.

use futures::channel::oneshot;
use gtk::cairo::{self, Format, ImageSurface};
use gtk::gio;
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

/// Wider boxes are scaled down to this many logical pixels, as on the Mac.
const MAX_WIDTH: f64 = 800.;

/// `rect` is x, y, width, height in the web view's logical pixels, from its
/// top left. The receiver gets None when WebKit couldn't draw it.
pub fn png(webview: &wry::WebView, rect: [f64; 4]) -> oneshot::Receiver<Option<Vec<u8>>> {
    use wry::WebViewExtUnix;
    let (tx, rx) = oneshot::channel();
    webview.webview().snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |shot| {
        let png = shot.ok().and_then(|shot| cut(&shot, rect)).and_then(|image| encode(image).ok());
        let _ = tx.send(png);
    });
    rx
}

/// The part of `shot` at `rect`, at most `MAX_WIDTH` wide, in the shot's pixels.
fn cut(shot: &cairo::Surface, [x, y, w, h]: [f64; 4]) -> Option<ImageSurface> {
    if w < 1. || h < 1. {
        return None;
    }
    let (scale, _) = shot.device_scale();
    let fit = (MAX_WIDTH / w).min(1.) * scale;
    let image = ImageSurface::create(Format::ARgb32, (w * fit).round() as i32, (h * fit).round() as i32).ok()?;
    let cr = cairo::Context::new(&image).ok()?;
    cr.scale(fit, fit);
    cr.set_source_surface(shot, -x, -y).ok()?;
    cr.paint().ok()?;
    drop(cr);
    image.flush();
    Some(image)
}

fn encode(mut image: ImageSurface) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (width, height, stride) = (image.width() as u32, image.height() as u32, image.stride() as usize);
    let data = image.data()?;
    // Cairo's ARGB32 is premultiplied, in native byte order (BGRA on little-endian).
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in data.chunks(stride).take(height as usize) {
        for px in row[..width as usize * 4].as_chunks::<4>().0 {
            let argb = u32::from_ne_bytes(*px);
            let a = (argb >> 24) as u8;
            let un = |c: u32| if a == 0 { 0 } else { ((c & 0xff) * 255 / a as u32).min(255) as u8 };
            rgba.extend_from_slice(&[un(argb >> 16), un(argb >> 8), un(argb), a]);
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgba)?;
    Ok(out)
}
