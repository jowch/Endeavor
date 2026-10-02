//! "Claude is waiting for you": a macOS notification when a prompt arrives
//! while Endeavor is in the background. Clicking it brings Endeavor forward,
//! and the workspace then opens the session it named (`take_clicked`).
//!
//! macOS's notification center only serves an app bundle with an identifier,
//! so a bare development binary sends nothing, and macOS refuses one run from a
//! temporary folder.

use std::sync::Mutex;

/// The session whose notification was clicked, until the workspace takes it.
static CLICKED: Mutex<Option<u64>> = Mutex::new(None);

const PREFIX: &str = "endeavor-ask-";

pub fn take_clicked() -> Option<u64> {
    CLICKED.lock().ok()?.take()
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Once;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "EndeavorNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
                let id = response.notification().request().identifier().to_string();
                if let Some(key) = id.strip_prefix(super::PREFIX).and_then(|k| k.parse().ok())
                    && let Ok(mut clicked) = super::CLICKED.lock()
                {
                    *clicked = Some(key);
                }
                done.call(());
            }
        }
    );

    fn bundled() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub fn send(title: &str, body: &str, key: u64) {
        if !bundled() {
            eprintln!("notification (not sent outside an app bundle): {title} — {body}");
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&format!("{}{key}", super::PREFIX)), &content, None);
        static SETUP: Once = Once::new();
        let mut first = false;
        SETUP.call_once(|| first = true);
        if !first {
            center.addNotificationRequest_withCompletionHandler(&request, None);
            return;
        }
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly; this one lives as long as the app.
        std::mem::forget(delegate);
        // A request added while macOS is still asking for permission is dropped,
        // so the first one goes once the answer is in.
        let asked = RcBlock::new(move |granted: Bool, _: *mut NSError| {
            if granted.as_bool() {
                UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &asked);
    }
}

/// Post "Claude is waiting for you" with the question and the session's title.
pub fn waiting(question: &str, session: &str, key: u64) {
    let body = format!("{} · {session}", question.replace('`', ""));
    #[cfg(target_os = "macos")]
    mac::send("Claude is waiting for you", &body, key);
    #[cfg(not(target_os = "macos"))]
    let _ = (body, key);
}
