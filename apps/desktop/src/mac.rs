//! The app's window on a Mac: WebKit's web view (WKWebView) through `wry`, its camera answered by the
//! page's address and its pages kept running while hidden, and a menu bar: without its
//! Edit menu no text could be copied or pasted in the pages, and without Quit the app couldn't be ended
//! from the keyboard.

use std::{
    collections::HashSet,
    ffi::{CStr, c_char},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use block2::Block;
use face_server::{
    app::{self, App},
    desktop,
};
use objc2::{
    ffi, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use tao::event_loop::EventLoopProxy;

use crate::{Told, log_written, note_title, title};

/// the app, for the answer to the camera's request (asked by WebKit on the window's own thread)
static APP: OnceLock<Arc<App>> = OnceLock::new();
/// each kind of request and its answer is said once
static SAID: Mutex<Option<HashSet<(String, isize, bool)>>> = Mutex::new(None);

/// WebKit's answers to "may this page use the camera?" (WKPermissionDecision)
const ASK_USER: isize = 0;
const GRANT: isize = 1;

/// WebKit's own switches that slow a covered or hidden window's pages, by their names in WebKit (private
/// ones: each is asked for, and left alone where a WebKit lacks it). Measured on the first real Mac
/// (2026-11-19): with another window over the app's the camera reached the other devices at 1
/// frame a second, and at 19.9 with the first of these off; a hidden or minimized window is helped part of
/// the way by the other three (timers 1.3 a second before, 13.5 to 15.7 after, 36 when shown).
const KEEP_RUNNING: [&str; 4] = [
    "WindowOcclusionDetectionEnabled",
    "HiddenPageDOMTimerThrottlingEnabled",
    "HiddenPageDOMTimerThrottlingAutoIncreases",
    "PageVisibilityBasedProcessSuppressionEnabled",
];

/// The window. Returns only with why it couldn't be made; closing it ends the app.
pub fn window(app: &Arc<App>, shell: &str, _home: &Path, shown: bool) -> String {
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        platform::macos::{ActivationPolicy, EventLoopExtMacOS},
        window::{UserAttentionType, WindowBuilder},
    };
    use wry::{WebViewBuilder, WebViewBuilderExtDarwin, WebViewExtMacOS};

    let _ = APP.set(app.clone());
    let mut event_loop = EventLoopBuilder::<Told>::with_user_event().build();
    if !shown {
        // (a check that shows nothing: no icon in the Dock either)
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let made = WindowBuilder::new()
        .with_title(app::TITLE)
        .with_inner_size(LogicalSize::new(1440.0, 920.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_visible(false) // (shown once its web view is there: never an empty window)
        .build(&event_loop);
    let window = match made {
        Ok(window) => window,
        Err(e) => return e.to_string(),
    };
    let menu = menu();
    let own = shell.trim_end_matches('/').to_string();
    let pages = app.clone();
    let died = event_loop.create_proxy();
    let built = WebViewBuilder::new()
        .with_url(shell)
        .with_background_color((0x0d, 0x15, 0x12, 255))
        // (the web view library's own answer to the camera's request, yes to every page, is replaced below:
        // should that fail, the answer is WebKit's own question, never yes)
        .with_permission_handler(|_| wry::PermissionResponse::Default)
        // the window shows the app's own pages and nothing else. WebKit asks here about every frame too (the
        // three programs' pages inside the tab page), so "the app's own" is asked of the app; a link
        // elsewhere goes to the owner's browser, but never from a window that isn't shown (a check)
        .with_navigation_handler(move |to| {
            let stays = pages.own_page(&to) || to.starts_with("about:");
            if !stays && shown && (to.starts_with("https://") || to.starts_with("http://")) {
                desktop::open_url(&to);
            }
            stays
        })
        // the page's own process ended (crashed, out of memory): WebKit leaves the window blank
        .with_on_web_content_process_terminate_handler(move || {
            let _ = died.send_event(Told::PageDied);
        })
        .build(&window);
    let webview = match built {
        Ok(webview) => webview,
        Err(e) => return e.to_string(),
    };
    let view = webview.webview();
    // SAFETY: the window's own web view, alive as long as `view`; only messages are sent to it
    let view_object: &AnyObject = unsafe { &*Retained::as_ptr(&view).cast::<AnyObject>() };
    answer_for_the_camera(view_object);
    keep_running(view_object);
    // alerts reach the window's loop from the app's own server; and the app watches its window from now on
    let proxy = Mutex::new(event_loop.create_proxy());
    app.set_window(app::Window {
        attention: Box::new(move |unread, flash| {
            let _ = proxy.lock().unwrap_or_else(|e| e.into_inner()).send_event(Told::Attention(unread, flash));
        }),
        before_exit: Box::new(log_written),
    });
    app.watch_window();
    window.set_visible(shown);
    println!("Window: open on {own}{}", if shown { "" } else { " (not shown)" });
    let later = event_loop.create_proxy();
    say_the_page_process_soon(&later);
    let app = app.clone();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep = (&webview, &menu); // (the web view and the menu live as long as the window's loop)
        match event {
            Event::UserEvent(Told::Attention(unread, flash)) => {
                window.set_title(&title(unread));
                note_title(unread);
                // (the Dock icon, until the app comes to the front)
                window.request_user_attention(flash.then_some(UserAttentionType::Critical));
            }
            Event::UserEvent(Told::PageDied) => {
                println!("The window's page process ended: loading the page again.");
                let _ = webview.reload();
                say_the_page_process_soon(&later);
            }
            Event::UserEvent(Told::PageProcess) => {
                // SAFETY: the window's own web view, on its own thread
                let view: &AnyObject = unsafe { &*Retained::as_ptr(&view).cast::<AnyObject>() };
                if let Some(pid) = page_process(view) {
                    println!("Window: its pages' process is {pid}");
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // in the log, so that an app that is gone can be told from one that crashed
                println!("The app window was closed: the app ends.");
                app.stop_tracker();
                log_written();
                std::process::exit(0);
            }
            // Quit, from the menu, the keyboard or the Dock
            Event::LoopDestroyed => {
                println!("The app was quit: the app ends.");
                app.stop_tracker();
                log_written();
            }
            _ => {}
        }
    })
}

/// The menu bar: the app's own menu (hide, quit), Edit (so that text in the pages can be cut, copied, pasted
/// and undone from the keyboard) and Window.
fn menu() -> Option<muda::Menu> {
    use muda::{Menu, PredefinedMenuItem as Item, Submenu};
    let build = || -> muda::Result<Menu> {
        let menu = Menu::new();
        let own = Submenu::with_items(
            app::TITLE,
            true,
            &[
                &Item::hide(None),
                &Item::hide_others(None),
                &Item::show_all(None),
                &Item::separator(),
                &Item::quit(None),
            ],
        )?;
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[
                &Item::undo(None),
                &Item::redo(None),
                &Item::separator(),
                &Item::cut(None),
                &Item::copy(None),
                &Item::paste(None),
                &Item::select_all(None),
            ],
        )?;
        let window = Submenu::with_items(
            "Window",
            true,
            &[&Item::minimize(None), &Item::maximize(None), &Item::separator(), &Item::close_window(None)],
        )?;
        menu.append_items(&[&own, &edit, &window])?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        Ok(menu)
    };
    match build() {
        Ok(menu) => Some(menu),
        Err(e) => {
            println!("Window: no menu bar ({e}).");
            None
        }
    }
}

/// A moment from now the log says which process the window's pages run in (WebKit starts it with the first
/// page): whoever reads the log about a page that stopped can look at that process.
fn say_the_page_process_soon(to: &EventLoopProxy<Told>) {
    let to = to.clone();
    let _ = std::thread::Builder::new().name("page process".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let _ = to.send_event(Told::PageProcess);
    });
}

fn responds(to: &AnyObject, name: Sel) -> bool {
    // SAFETY: every object answers this
    unsafe { msg_send![to, respondsToSelector: name] }
}

/// The process the web view's pages run in, where this WebKit says (a private name).
fn page_process(view: &AnyObject) -> Option<i32> {
    if !responds(view, sel!(_webProcessIdentifier)) {
        return None;
    }
    // SAFETY: asked for above; it takes nothing and gives a process number
    let pid: i32 = unsafe { msg_send![view, _webProcessIdentifier] };
    (pid > 0).then_some(pid)
}

/// The window's pages keep their pace when it is covered or hidden (`KEEP_RUNNING`): set on the web view
/// and its preferences before any page runs, and said in the log, with what this macOS lacks.
fn keep_running(view: &AnyObject) {
    // SAFETY: the web view's own configuration and its preferences; each switch is asked for before it is
    // set, and takes one yes-or-no
    let had: [bool; 4] = unsafe {
        let configuration: Retained<AnyObject> = msg_send![view, configuration];
        let preferences: Retained<AnyObject> = msg_send![&*configuration, preferences];
        let preferences: &AnyObject = &preferences;
        let occlusion = responds(view, sel!(_setWindowOcclusionDetectionEnabled:));
        if occlusion {
            let _: () = msg_send![view, _setWindowOcclusionDetectionEnabled: false];
        }
        let throttling = responds(preferences, sel!(_setHiddenPageDOMTimerThrottlingEnabled:));
        if throttling {
            let _: () = msg_send![preferences, _setHiddenPageDOMTimerThrottlingEnabled: false];
        }
        let increases = responds(preferences, sel!(_setHiddenPageDOMTimerThrottlingAutoIncreases:));
        if increases {
            let _: () = msg_send![preferences, _setHiddenPageDOMTimerThrottlingAutoIncreases: false];
        }
        let suppression = responds(preferences, sel!(_setPageVisibilityBasedProcessSuppressionEnabled:));
        if suppression {
            let _: () = msg_send![preferences, _setPageVisibilityBasedProcessSuppressionEnabled: false];
        }
        [occlusion, throttling, increases, suppression]
    };
    let names = |want: bool| -> Vec<&str> {
        KEEP_RUNNING.iter().zip(had).filter(|(_, had)| *had == want).map(|(name, _)| *name).collect()
    };
    let (done, missing) = (names(true), names(false));
    println!(
        "Window: its pages keep their pace when it is covered or hidden ({}: off){}",
        done.join(", "),
        if missing.is_empty() { String::new() } else { format!("; this macOS lacks {}", missing.join(", ")) }
    );
}

/// A WebKit origin as an address (`http://127.0.0.1:8764/`).
fn origin_address(origin: &AnyObject) -> Option<String> {
    // SAFETY: a WKSecurityOrigin's own three values; the texts are copied while their objects live
    unsafe {
        let text = |of: Option<Retained<AnyObject>>| -> Option<String> {
            let of = of?;
            let bytes: *const c_char = msg_send![&*of, UTF8String];
            (!bytes.is_null()).then(|| CStr::from_ptr(bytes).to_string_lossy().into_owned())
        };
        let protocol = text(msg_send![origin, protocol])?;
        let host = text(msg_send![origin, host])?;
        let port: isize = msg_send![origin, port];
        Some(format!("{protocol}://{host}:{port}/"))
    }
}

/// WebKit's question to its window, `webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:
/// decisionHandler:`, answered: yes for the tracker this process started and the tab page around it
/// (`App::camera_allowed`), and for any other page WebKit's own question. The system's own question for the
/// app (once, remembered) stays.
extern "C-unwind" fn camera_asked(
    _delegate: &AnyObject,
    _name: Sel,
    _view: &AnyObject,
    origin: &AnyObject,
    _frame: &AnyObject,
    kind: isize,
    answer: &Block<dyn Fn(isize)>,
) {
    let address = origin_address(origin);
    let allowed = match (&address, APP.get()) {
        (Some(address), Some(app)) => (0..=2).contains(&kind) && app.camera_allowed(address),
        _ => false,
    };
    let address = address.unwrap_or_else(|| "a page".to_string());
    let new = SAID.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert((
        address.clone(),
        kind,
        allowed,
    ));
    if new {
        let what = match kind {
            0 => "the camera",
            1 => "the microphone",
            2 => "the camera and the microphone",
            _ => "a device",
        };
        println!(
            "Permission: {address} asked for {what}: {}",
            if allowed { "allowed" } else { "left to the window to ask" }
        );
    }
    answer.call((if allowed { GRANT } else { ASK_USER },));
}

/// The window answers the tracker's camera and microphone request itself (`camera_asked`). WebKit asks its
/// window about every page that wants them and never remembers the answer, so a window that asked the
/// person would ask at every start and every time the page is loaded again, with nobody there to answer.
/// The web view library's own answer is yes to every page; it is replaced by this one, on the library's
/// own class for the web view's delegate.
fn answer_for_the_camera(view: &AnyObject) {
    type Asked =
        extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &AnyObject, &AnyObject, isize, &Block<dyn Fn(isize)>);
    let name = sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:);
    // SAFETY: the web view's own delegate; the function has the method's own shape (an object, the origin,
    // the frame, the kind as a number, a block that takes a number), which the types' text says too
    let replaced = unsafe {
        let delegate: Option<Retained<AnyObject>> = msg_send![view, UIDelegate];
        match delegate {
            Some(delegate) if responds(&delegate, name) => {
                let class: *const AnyClass = delegate.class();
                let function = std::mem::transmute::<Asked, Imp>(camera_asked as Asked);
                ffi::class_replaceMethod(class.cast_mut(), name, function, c"v@:@@@q@?".as_ptr()).is_some()
            }
            _ => false,
        }
    };
    if !replaced {
        println!("Window: the camera's request can't be answered here: the window will ask each time its page loads.");
    }
}
