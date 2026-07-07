//! Push-to-talk global shortcut registration.
//!
//! Two capture paths:
//! - Combos with a regular key ("Alt+Space") go through
//!   tauri-plugin-global-shortcut, as system accelerators.
//! - Modifier-only combos ("Shift+Alt") cannot be expressed as accelerators,
//!   so on macOS they are detected with a listen-only CGEventTap on
//!   flagsChanged events (requires the Accessibility permission the app
//!   already holds for synthetic paste).

use crate::dictation;
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);

enum Registered {
    Plugin(Shortcut),
    // The tap is never read, only kept alive; dropping it tears the tap down.
    #[cfg(target_os = "macos")]
    ModifierTap(#[allow(dead_code)] modifier_tap::ModifierTap),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

fn modifier_token(token: &str) -> Option<Modifier> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifier::Ctrl),
        "alt" | "option" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        "super" | "cmd" | "command" | "meta" => Some(Modifier::Super),
        _ => None,
    }
}

/// Returns the deduplicated modifier set when the combo consists of
/// modifier tokens only, `None` when it contains a regular key.
fn parse_modifier_only(hotkey: &str) -> Option<Vec<Modifier>> {
    let mut mods = Vec::new();
    for token in hotkey.split('+') {
        let m = modifier_token(token.trim())?;
        if !mods.contains(&m) {
            mods.push(m);
        }
    }
    (!mods.is_empty()).then_some(mods)
}

pub fn register(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let mut guard = REGISTERED.lock().unwrap();
    unregister_locked(app, &mut guard);
    *guard = Some(build(app, hotkey)?);
    log::info!("[hotkey] registered \"{hotkey}\"");
    Ok(())
}

/// Swaps the registered hotkey; restores the old one if the new registration fails.
pub fn replace(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let mut guard = REGISTERED.lock().unwrap();
    unregister_locked(app, &mut guard);

    match build(app, new) {
        Ok(entry) => {
            *guard = Some(entry);
            log::info!("[hotkey] registered \"{new}\"");
            Ok(())
        }
        Err(e) => {
            match build(app, old) {
                Ok(entry) => *guard = Some(entry),
                Err(restore) => {
                    log::error!("[hotkey] failed to restore previous hotkey: {restore}")
                }
            }
            Err(e)
        }
    }
}

fn unregister_locked(app: &AppHandle, guard: &mut Option<Registered>) {
    match guard.take() {
        Some(Registered::Plugin(shortcut)) => {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        // Dropping the tap stops its run loop and joins the thread.
        #[cfg(target_os = "macos")]
        Some(Registered::ModifierTap(_)) => {}
        None => {}
    }
}

fn build(app: &AppHandle, hotkey: &str) -> Result<Registered, String> {
    if let Some(mods) = parse_modifier_only(hotkey) {
        if mods.len() < 2 {
            return Err(format!(
                "modifier-only hotkey \"{hotkey}\" needs at least two modifier keys \
                 (a single modifier would fire during normal typing)"
            ));
        }
        #[cfg(target_os = "macos")]
        {
            let tap = modifier_tap::ModifierTap::spawn(app.clone(), &mods)?;
            return Ok(Registered::ModifierTap(tap));
        }
        #[cfg(not(target_os = "macos"))]
        return Err(format!(
            "modifier-only hotkey \"{hotkey}\" is only supported on macOS"
        ));
    }

    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| format!("invalid hotkey \"{hotkey}\": {e}"))?;

    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            log::debug!("[hotkey] event: {:?}", event.state);
            match event.state {
                ShortcutState::Pressed => dictation::hotkey_pressed(app),
                ShortcutState::Released => dictation::hotkey_released(app),
            }
        })
        .map_err(|e| format!("register hotkey \"{hotkey}\": {e}"))?;

    Ok(Registered::Plugin(shortcut))
}

#[cfg(target_os = "macos")]
mod modifier_tap {
    use super::Modifier;
    use crate::dictation;
    use core_foundation::base::TCFType;
    use core_foundation::mach_port::CFMachPortRef;
    use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, CallbackResult,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use tauri::AppHandle;

    // core-graphics does not re-export this; needed to revive the tap after
    // the system disables it (kCGEventTapDisabledByTimeout).
    extern "C" {
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    }

    fn target_flags(mods: &[Modifier]) -> CGEventFlags {
        mods.iter().fold(CGEventFlags::empty(), |acc, m| {
            acc | match m {
                Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
                Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
                Modifier::Shift => CGEventFlags::CGEventFlagShift,
                Modifier::Super => CGEventFlags::CGEventFlagCommand,
            }
        })
    }

    fn modifier_mask() -> CGEventFlags {
        CGEventFlags::CGEventFlagControl
            | CGEventFlags::CGEventFlagAlternate
            | CGEventFlags::CGEventFlagShift
            | CGEventFlags::CGEventFlagCommand
    }

    /// Listen-only CGEventTap running a CFRunLoop on a dedicated thread.
    /// Dropping it stops the run loop and joins the thread.
    pub struct ModifierTap {
        stop: Arc<AtomicBool>,
        runloop: CFRunLoop,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ModifierTap {
        pub fn spawn(app: AppHandle, mods: &[Modifier]) -> Result<Self, String> {
            let target = target_flags(mods);
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<CFRunLoop, String>>();

            let thread = std::thread::Builder::new()
                .name("hotkey-modifier-tap".into())
                .spawn(move || tap_thread(app, target, thread_stop, ready_tx))
                .map_err(|e| format!("spawn event-tap thread: {e}"))?;

            match ready_rx.recv() {
                Ok(Ok(runloop)) => Ok(Self {
                    stop,
                    runloop,
                    thread: Some(thread),
                }),
                Ok(Err(e)) => {
                    let _ = thread.join();
                    Err(e)
                }
                Err(_) => {
                    let _ = thread.join();
                    Err("event-tap thread exited unexpectedly".into())
                }
            }
        }
    }

    impl Drop for ModifierTap {
        fn drop(&mut self) {
            // The flag (checked between run-loop slices) guarantees the
            // thread exits even if stop() lands before the loop is entered,
            // in which case CFRunLoopStop is a no-op.
            self.stop.store(true, Ordering::SeqCst);
            self.runloop.stop();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn tap_thread(
        app: AppHandle,
        target: CGEventFlags,
        stop: Arc<AtomicBool>,
        ready_tx: mpsc::Sender<Result<CFRunLoop, String>>,
    ) {
        // Whether the target combo is currently held; keeps pressed/released
        // events idempotent while flags fluctuate above the target set.
        let active = AtomicBool::new(false);
        // Raw CFMachPortRef of the tap, filled in after creation so the
        // callback can re-enable the tap if the system disables it. Only
        // touched from this thread.
        let port = Arc::new(AtomicUsize::new(0));
        let port_cb = port.clone();

        let tap = match CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged],
            move |_proxy, etype, event| {
                handle_event(&app, etype, event, target, &active, &port_cb);
                CallbackResult::Keep
            },
        ) {
            Ok(tap) => tap,
            Err(()) => {
                let _ = ready_tx.send(Err(
                    "failed to create the keyboard event tap (is the Accessibility permission granted?)"
                        .into(),
                ));
                return;
            }
        };

        let Ok(source) = tap.mach_port().create_runloop_source(0) else {
            let _ = ready_tx.send(Err("failed to create a run-loop source for the event tap".into()));
            return;
        };

        port.store(
            tap.mach_port().as_concrete_TypeRef() as usize,
            Ordering::SeqCst,
        );
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
        tap.enable();

        let _ = ready_tx.send(Ok(CFRunLoop::get_current()));
        while !stop.load(Ordering::SeqCst) {
            CFRunLoop::run_in_mode(
                unsafe { kCFRunLoopDefaultMode },
                std::time::Duration::from_millis(500),
                false,
            );
        }
        // The tap is dropped (and the mach port invalidated) on return.
    }

    fn handle_event(
        app: &AppHandle,
        etype: CGEventType,
        event: &CGEvent,
        target: CGEventFlags,
        active: &AtomicBool,
        port: &AtomicUsize,
    ) {
        match etype {
            CGEventType::FlagsChanged => {}
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
                log::warn!("[hotkey] event tap disabled by the system; re-enabling");
                let raw = port.load(Ordering::SeqCst);
                if raw != 0 {
                    unsafe { CGEventTapEnable(raw as CFMachPortRef, true) };
                }
                // Modifiers may have been released while the tap was dead.
                if active.swap(false, Ordering::SeqCst) {
                    fire(app, false);
                }
                return;
            }
            _ => return,
        }

        let held = event.get_flags() & modifier_mask();
        if held.contains(target) {
            if !active.swap(true, Ordering::SeqCst) {
                fire(app, true);
            }
        } else if active.swap(false, Ordering::SeqCst) {
            fire(app, false);
        }
    }

    /// Dispatches to the main thread so the dictation state machine runs in
    /// the same context as the global-shortcut plugin path, and the tap
    /// callback stays fast (slow callbacks get the tap disabled).
    fn fire(app: &AppHandle, pressed: bool) {
        log::debug!(
            "[hotkey] modifier combo {}",
            if pressed { "pressed" } else { "released" }
        );
        let app = app.clone();
        let _ = app.clone().run_on_main_thread(move || {
            if pressed {
                dictation::hotkey_pressed(&app);
            } else {
                dictation::hotkey_released(&app);
            }
        });
    }
}
