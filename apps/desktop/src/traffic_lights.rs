use std::cell::Cell;

use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Weak, sel};
use objc2_app_kit::{
    NSWindow, NSWindowButton, NSWindowDidBecomeKeyNotification,
    NSWindowDidExitFullScreenNotification, NSWindowDidResignKeyNotification,
    NSWindowDidResizeNotification, NSWindowDidUpdateNotification, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSPoint};

// The buttons center on the 48pt top bar, inset from the left by the same 24pt.
// AppKit's own spacing between them is kept (23pt on macOS 26), so the toggle
// sits 32pt after the last button's center: system.html places it at (86, 8).
const CENTER: f64 = 24.0;
static LAYOUT_KEY: u8 = 0;

struct LayoutState {
    window: Weak<NSWindow>,
    pitch: f64,
    applying: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LayoutState]
    struct TrafficLightLayout;

    impl TrafficLightLayout {
        #[unsafe(method(windowUpdated:))]
        fn window_updated(&self, _notification: &NSNotification) {
            self.layout();
        }
    }
);

impl TrafficLightLayout {
    fn layout(&self) {
        let state = self.ivars();
        let Some(window) = state.window.load() else {
            return;
        };
        // AppKit owns the auto-hiding fullscreen title bar and its transition windows.
        if window.styleMask().contains(NSWindowStyleMask::FullScreen)
            || state.applying.replace(true)
        {
            return;
        }
        self.position_buttons(&window);
        state.applying.set(false);
    }

    fn position_buttons(&self, window: &NSWindow) {
        let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
            return;
        };
        let Some(content) = window.contentView() else {
            return;
        };
        // SAFETY: standard window buttons and their title-bar views are AppKit NSViews.
        let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) })
        else {
            return;
        };
        // Overlay keeps content at the window's top. Give the native title bar the same
        // 48pt center line, rather than interpreting Wry's inset as a top-left position.
        let mut frame = container.frame();
        frame.size.height = CENTER * 2.0;
        frame.origin.y = window.frame().size.height - frame.size.height;
        if container.frame() != frame {
            container.setFrame(frame);
        }
        for (index, kind) in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ]
        .into_iter()
        .enumerate()
        {
            let Some(button) = window.standardWindowButton(kind) else {
                continue;
            };
            let Some(parent) = (unsafe { button.superview() }) else {
                continue;
            };
            let center = parent.convertPoint_fromView(
                NSPoint::new(
                    CENTER + index as f64 * self.ivars().pitch,
                    content.bounds().size.height - CENTER,
                ),
                Some(&content),
            );
            let mut frame = button.frame();
            frame.origin = NSPoint::new(
                center.x - frame.size.width / 2.0,
                center.y - frame.size.height / 2.0,
            );
            if button.frame() != frame {
                button.setFrameOrigin(frame.origin);
            }
        }
    }
}

pub fn install(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let mtm = MainThreadMarker::new().expect("window setup runs on the main thread");
    // SAFETY: Tauri returns this window's NSWindow; setup and notifications run on main.
    unsafe {
        let native = &*window.ns_window()?.cast::<NSWindow>();
        // Read the spacing before the first layout pass moves the buttons.
        let origin = |kind| {
            native
                .standardWindowButton(kind)
                .map(|button| button.frame().origin.x)
        };
        let pitch = match (
            origin(NSWindowButton::CloseButton),
            origin(NSWindowButton::MiniaturizeButton),
        ) {
            (Some(close), Some(miniaturize)) if miniaturize > close => miniaturize - close,
            _ => 23.0,
        };
        let observer = TrafficLightLayout::alloc(mtm).set_ivars(LayoutState {
            window: Weak::from(native),
            pitch,
            applying: Cell::new(false),
        });
        let observer: objc2::rc::Retained<TrafficLightLayout> = msg_send![super(observer), init];
        let center = NSNotificationCenter::defaultCenter();
        for name in [
            NSWindowDidUpdateNotification,
            NSWindowDidResizeNotification,
            NSWindowDidBecomeKeyNotification,
            NSWindowDidResignKeyNotification,
            NSWindowDidExitFullScreenNotification,
        ] {
            center.addObserver_selector_name_object(
                &observer,
                sel!(windowUpdated:),
                Some(name),
                Some(native),
            );
        }
        // The window owns its observer; the observer only holds a weak window reference.
        // Selector observers are zeroing references in NSNotificationCenter (macOS 10.11+).
        objc2::ffi::objc_setAssociatedObject(
            (native as *const NSWindow).cast_mut().cast(),
            (&LAYOUT_KEY as *const u8).cast(),
            objc2::rc::Retained::as_ptr(&observer).cast_mut().cast(),
            objc2::ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
        observer.layout();
    }
    Ok(())
}
