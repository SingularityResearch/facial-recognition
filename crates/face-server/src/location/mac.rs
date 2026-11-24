//! A Mac's own location service (Core Location), asked only while the location is wanted (`super::wanted`:
//! the page's switch is on and no place is typed).
//!
//! Core Location answers on the thread that made its manager, through that thread's run loop: so the manager,
//! its delegate and a run loop live on one thread of their own, started the first time the location is
//! wanted. macOS asks the owner once whether the app may see the location (the words are the app's
//! `NSLocationUsageDescription`), and the signed app carries the entitlement for it. What the system says
//! goes to `super::told`, and why it says nothing to `super::refused`; nothing else is made of it here.

use std::{ffi::c_void, sync::Once, time::Duration};

use objc2::{
    AllocAnyThread, Encode, Encoding, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject},
};

use super::{NOT_ALLOWED, NOT_FOUND, SWITCHED_OFF};

#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    /// Runs the calling thread's run loop for at most `seconds`; 1: it has nothing to wait for.
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
}

/// `CLLocationCoordinate2D`: degrees north and east.
#[repr(C)]
#[derive(Clone, Copy)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

// SAFETY: two doubles, as Core Location declares the structure
unsafe impl Encode for Coordinate {
    const ENCODING: Encoding = Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}

/// `kCLErrorDenied`: the owner, or a rule of the Mac, said no.
const DENIED: isize = 1;
/// `CLAuthorizationStatus`: restricted, denied.
const RESTRICTED: i32 = 1;
const REFUSED: i32 = 2;

/// A `CLLocation` as the app takes it: only one whose accuracy the system knows (a negative one is no
/// location).
fn take(location: &AnyObject) {
    // SAFETY: a CLLocation answers both with plain numbers
    let (at, accuracy): (Coordinate, f64) =
        unsafe { (msg_send![location, coordinate], msg_send![location, horizontalAccuracy]) };
    if accuracy >= 0.0 {
        super::told(at.latitude, at.longitude, accuracy);
    }
}

define_class!(
    // SAFETY: NSObject has no rules for its subclasses, and the class keeps nothing of its own
    #[unsafe(super(NSObject))]
    #[name = "FaceRecognizerLocationDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn updated(&self, _manager: &AnyObject, locations: &AnyObject) {
            // SAFETY: an NSArray of CLLocation, the newest last
            let last: Option<Retained<AnyObject>> = unsafe { msg_send![locations, lastObject] };
            if let Some(location) = last {
                take(&location);
            }
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn failed(&self, _manager: &AnyObject, error: &AnyObject) {
            // SAFETY: an NSError's code is a number
            let code: isize = unsafe { msg_send![error, code] };
            super::refused(i32::from(if code == DENIED { NOT_ALLOWED } else { NOT_FOUND }));
        }

        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn allowed(&self, manager: &AnyObject) {
            // SAFETY: the manager's own answer, a number
            let status: i32 = unsafe { msg_send![manager, authorizationStatus] };
            if status == RESTRICTED || status == REFUSED {
                super::refused(i32::from(NOT_ALLOWED));
            }
        }
    }
);

impl Delegate {
    fn new() -> Retained<Delegate> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's own init
        unsafe { msg_send![super(this), init] }
    }
}

/// The location is wanted: Core Location's thread is started, once. It asks and stops asking by itself as
/// the wish changes.
pub fn ask() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("Core Location".into()).spawn(run);
    });
}

fn run() {
    // SAFETY: Core Location's manager is made, used and kept on this one thread, whose run loop it answers on;
    // every message is one the class declares, with the argument and answer it declares
    unsafe {
        let manager: Retained<AnyObject> = msg_send![class!(CLLocationManager), new];
        let delegate = Delegate::new();
        let _: () = msg_send![&*manager, setDelegate: &*delegate];
        // (kCLLocationAccuracyBest: as exact as the Mac can say)
        let _: () = msg_send![&*manager, setDesiredAccuracy: -1.0f64];
        let mut asking = false;
        loop {
            let wanted = super::wanted();
            if wanted && !asking {
                let on: bool = msg_send![class!(CLLocationManager), locationServicesEnabled];
                if on {
                    let _: () = msg_send![&*manager, requestWhenInUseAuthorization];
                    let _: () = msg_send![&*manager, startUpdatingLocation];
                    asking = true;
                } else {
                    super::refused(i32::from(SWITCHED_OFF));
                }
            } else if !wanted && asking {
                let _: () = msg_send![&*manager, stopUpdatingLocation];
                asking = false;
            } else if wanted && asking {
                // (what the manager knows already, for a Mac that hasn't moved since it was last told)
                let known: Option<Retained<AnyObject>> = msg_send![&*manager, location];
                if let Some(location) = known {
                    take(&location);
                }
            }
            // the manager's answers arrive while the run loop runs; a run loop with nothing to wait for
            // comes back at once, so the thread rests itself then
            if CFRunLoopRunInMode(kCFRunLoopDefaultMode, 2.0, 0) == 1 {
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}
