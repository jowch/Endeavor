//! Linux's `src/webcontent.rs`: WebKitGTK's web process ending, and the page's address.

use webkit2gtk::WebViewExt;

pub fn on_process_ended(webview: &wry::WebView, ended: impl Fn() + 'static) {
    use wry::WebViewExtUnix;
    webview.webview().connect_web_process_terminated(move |_, reason| {
        eprintln!("WebKitGTK's web process ended: {reason:?}");
        ended();
    });
}

pub fn url(webview: &wry::WebView) -> String {
    webview.url().unwrap_or_default()
}
