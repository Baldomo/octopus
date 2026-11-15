//! iOS suspends an app within seconds of it leaving the front unless it holds a background task, and lets a held task
//! run for about 30 s. The app holds one from leaving the front until it comes back or the task expires, so its
//! connections and terminal streams outlive a quick trip to another app.
#![allow(unsafe_code, reason = "UIKit's notification names and invalid task are extern statics")]

use std::cell::Cell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};
use objc2_ui_kit::{
    UIApplication, UIApplicationDidEnterBackgroundNotification, UIApplicationWillEnterForegroundNotification,
    UIBackgroundTaskIdentifier, UIBackgroundTaskInvalid,
};

thread_local! {
    static HELD: Cell<UIBackgroundTaskIdentifier> = Cell::new(invalid());
}

pub(crate) fn follow() {
    let left = RcBlock::new(|_: NonNull<NSNotification>| hold());
    let back = RcBlock::new(|_: NonNull<NSNotification>| release());
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: UIKit's notification names are static NSStrings.
    let names = unsafe { [UIApplicationDidEnterBackgroundNotification, UIApplicationWillEnterForegroundNotification] };
    for (name, block) in names.into_iter().zip([&left, &back]) {
        // SAFETY: the main queue runs the block on the main thread, which owns the held task.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&NSOperationQueue::mainQueue()),
                block,
            )
        };
        std::mem::forget(observer);
    }
}

fn hold() {
    release();
    let expired = RcBlock::new(release);
    HELD.set(app().beginBackgroundTaskWithExpirationHandler(Some(&expired)));
}

/// UIKit calls a task's expiry handler on the main thread too.
fn release() {
    let held = HELD.replace(invalid());
    if held != invalid() {
        app().endBackgroundTask(held);
    }
}

fn app() -> objc2::rc::Retained<UIApplication> {
    UIApplication::sharedApplication(MainThreadMarker::new().expect("the main thread holds and releases the task"))
}

fn invalid() -> UIBackgroundTaskIdentifier {
    // SAFETY: UIKit's invalid task identifier is a constant.
    unsafe { UIBackgroundTaskInvalid }
}
