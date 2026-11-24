use std::sync::Mutex;

static CLICKED: Mutex<Option<u64>> = Mutex::new(None);

#[cfg(target_os = "macos")]
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
        std::mem::forget(delegate);
        let asked = RcBlock::new(move |granted: Bool, _: *mut NSError| {
            if granted.as_bool() {
                UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &asked);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::HashMap;

    use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
    use gtk::gio::{self, DBusCallFlags, DBusConnection, DBusSignalFlags};
    use gtk::glib::{self, ToVariant, Variant, VariantTy};

    const NAME: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";

    thread_local! {
        static BUS: OnceCell<Option<DBusConnection>> = const { OnceCell::new() };
        static SHOWN: Cell<Option<(u32, u64)>> = const { Cell::new(None) };
        static CLICKS: RefCell<Option<UnboundedSender<()>>> = const { RefCell::new(None) };
    }

    pub fn clicks() -> UnboundedReceiver<()> {
        let (tx, rx) = unbounded();
        CLICKS.with(|c| *c.borrow_mut() = Some(tx));
        rx
    }

    fn bus() -> Option<DBusConnection> {
        BUS.with(|bus| {
            bus.get_or_init(|| {
                let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(|e| eprintln!("notification: no session bus: {e}")).ok()?;
                bus.signal_subscribe(Some(NAME), Some(NAME), Some("ActionInvoked"), Some(PATH), None, DBusSignalFlags::NONE, |_, _, _, _, _, params| {
                    let Some((id, _action)) = params.get::<(u32, String)>() else { return };
                    let Some((shown, key)) = SHOWN.get() else { return };
                    if id != shown {
                        return;
                    }
                    if let Ok(mut clicked) = super::CLICKED.lock() {
                        *clicked = Some(key);
                    }
                    CLICKS.with(|c| {
                        if let Some(tx) = c.borrow().as_ref() {
                            let _ = tx.unbounded_send(());
                        }
                    });
                });
                Some(bus)
            })
            .clone()
        })
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn send(title: &str, body: &str, key: u64) {
        let Some(bus) = bus() else { return };
        let actions = vec!["default".to_string(), "Open".to_string()];
        let hints: HashMap<String, Variant> = HashMap::new();
        let replaces = SHOWN.get().map_or(0, |(id, _)| id);
        let params = ("Endeavor", replaces, "", title, escape(body), actions, hints, -1i32).to_variant();
        bus.call(Some(NAME), PATH, NAME, "Notify", Some(&params), Some(VariantTy::new("(u)").unwrap()), DBusCallFlags::NONE, -1, None::<&gio::Cancellable>, move |reply: Result<Variant, glib::Error>| match reply {
            Ok(reply) => {
                if let Some((id,)) = reply.get::<(u32,)>() {
                    SHOWN.set(Some((id, key)));
                }
            }
            Err(e) => eprintln!("notification not sent: {e}"),
        });
    }
}

#[cfg(target_os = "linux")]
pub use linux::clicks;

#[cfg(windows)]
mod windows {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows_sys::Win32::UI::WindowsAndMessaging::{FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx};

    static WINDOW: AtomicUsize = AtomicUsize::new(0);

    pub fn set_window(window: &gpui::Window) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        if let RawWindowHandle::Win32(handle) = handle.as_raw() {
            WINDOW.store(handle.hwnd.get() as usize, Ordering::Relaxed);
        }
    }

    pub fn flash() {
        let hwnd = WINDOW.load(Ordering::Relaxed);
        if hwnd == 0 {
            return;
        }
        let info = FLASHWINFO { cbSize: size_of::<FLASHWINFO>() as u32, hwnd: hwnd as _, dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
        unsafe { FlashWindowEx(&info) };
    }
}

#[cfg(windows)]
pub use windows::set_window;

pub fn waiting(agent: &str, question: &str, session: &str, key: u64) {
    let body = format!("{} · {session}", question.replace('`', ""));
    #[cfg(target_os = "macos")]
    mac::send(&format!("{agent} is waiting for you"), &body, key);
    #[cfg(target_os = "linux")]
    linux::send(&format!("{agent} is waiting for you"), &body, key);
    #[cfg(windows)]
    {
        let _ = (agent, body, key);
        windows::flash();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use gtk::gio::{self, DBusConnectionFlags, DBusNodeInfo};
    use gtk::glib::{self, ToVariant};

    const NOTIFY: &str = r#"<node><interface name="org.freedesktop.Notifications"><method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method></interface></node>"#;

    fn spin(ctx: &glib::MainContext) {
        for _ in 0..50 {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "needs its own session bus (dbus-run-session)"]
    fn a_click_names_its_session() {
        let ctx = glib::MainContext::default();
        let _owner = ctx.acquire().unwrap();
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("run under dbus-run-session");
        let server = gio::DBusConnection::for_address_sync(&address, DBusConnectionFlags::AUTHENTICATION_CLIENT | DBusConnectionFlags::MESSAGE_BUS_CONNECTION, None, None::<&gio::Cancellable>).unwrap();
        let info = DBusNodeInfo::for_xml(NOTIFY).unwrap();
        let iface = info.lookup_interface("org.freedesktop.Notifications").unwrap();
        let body = Arc::new(std::sync::Mutex::new(String::new()));
        let seen = body.clone();
        server
            .register_object(
                "/org/freedesktop/Notifications",
                &iface,
                move |_, _, _, _, _, params, call| {
                    *seen.lock().unwrap() = params.child_value(4).get::<String>().unwrap();
                    call.return_value(Some(&(7u32,).to_variant()));
                },
                |_, _, _, _, _| unreachable!(),
                |_, _, _, _, _, _| false,
            )
            .unwrap();
        let owned = Arc::new(AtomicBool::new(false));
        let flag = owned.clone();
        let _name = gio::bus_own_name_on_connection(&server, "org.freedesktop.Notifications", gio::BusNameOwnerFlags::NONE, move |_, _| flag.store(true, Ordering::SeqCst), |_, _| panic!("lost the name"));
        while !owned.load(Ordering::SeqCst) {
            ctx.iteration(true);
        }
        let mut clicks = super::clicks();
        super::waiting("Claude", "Run `x` <now> & then?", "My session", 42);
        spin(&ctx);
        assert_eq!(*body.lock().unwrap(), "Run x &lt;now&gt; &amp; then? · My session");

        let click = |id: u32| server.emit_signal(None, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked", Some(&(id, "default").to_variant())).unwrap();
        click(8);
        spin(&ctx);
        assert_eq!(super::take_clicked(), None);
        click(7);
        spin(&ctx);
        assert_eq!(super::take_clicked(), Some(42));
        assert!(clicks.try_recv().is_ok());
    }
}
