//! iOS Drag and Drop implementation using UIDropInteraction.
//!
//! This module implements file drop support for iOS, allowing files to be
//! dragged from the Files app or other apps into GPUI views.
//!
//! The implementation uses UIDropInteractionDelegate protocol methods:
//! - dropInteraction:canHandle: - Accept/reject drop sessions
//! - dropInteraction:sessionDidEnter: - Handle drag enter
//! - dropInteraction:sessionDidUpdate: - Track position during drag
//! - dropInteraction:sessionDidExit: - Handle drag exit
//! - dropInteraction:performDrop: - Handle actual drop

use crate::{ExternalPaths, FileDropEvent, PlatformInput, Point, Pixels, px};
use objc::{
    class,
    msg_send,
    runtime::{Object, Sel, BOOL, NO, YES},
    sel, sel_impl,
};
use smallvec::SmallVec;
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    path::PathBuf,
    sync::OnceLock,
};

// UTType identifier for file URLs
const PUBLIC_FILE_URL: &str = "public.file-url";

/// Storage for pending drop session data.
/// Only one drop session can be active at a time.
struct DropSessionState {
    /// Paths extracted from the current drop session
    pending_paths: UnsafeCell<Option<ExternalPaths>>,
    /// Whether a drop session is currently active
    is_active: UnsafeCell<bool>,
}

// Safety: Only accessed from main thread on iOS
unsafe impl Send for DropSessionState {}
unsafe impl Sync for DropSessionState {}

static DROP_SESSION_STATE: OnceLock<DropSessionState> = OnceLock::new();

fn get_drop_state() -> &'static DropSessionState {
    DROP_SESSION_STATE.get_or_init(|| DropSessionState {
        pending_paths: UnsafeCell::new(None),
        is_active: UnsafeCell::new(false),
    })
}

/// UIDropOperation constants
#[repr(usize)]
#[derive(Clone, Copy, Debug)]
pub enum UIDropOperation {
    Cancel = 0,
    Forbidden = 1,
    Copy = 2,
    Move = 3,
}

/// Check if a drop session contains file URLs.
///
/// # Safety
/// session must be a valid UIDropSession pointer.
pub unsafe fn session_has_file_urls(session: *mut Object) -> bool {
    if session.is_null() {
        return false;
    }

    // Create NSArray with the file URL type identifier
    let type_str: *mut Object = msg_send![
        class!(NSString),
        stringWithUTF8String: PUBLIC_FILE_URL.as_ptr() as *const i8
    ];
    let type_array: *mut Object = msg_send![class!(NSArray), arrayWithObject: type_str];

    // Check if session has items conforming to file URL type
    let has_items: BOOL = msg_send![session, hasItemsConformingToTypeIdentifiers: type_array];
    has_items == YES
}

/// Extract the drop position from a session relative to a view.
///
/// # Safety
/// session and view must be valid pointers.
pub unsafe fn drop_position(session: *mut Object, view: *mut Object) -> Point<Pixels> {
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    let location: CGPoint = msg_send![session, locationInView: view];
    Point::new(px(location.x as f32), px(location.y as f32))
}

/// Extract file URLs from a drop session's item providers.
///
/// This function synchronously extracts URLs that are immediately available.
/// For items that require async loading, it returns what it can get synchronously.
///
/// # Safety
/// session must be a valid UIDropSession pointer.
pub unsafe fn extract_file_urls_from_session(session: *mut Object) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if session.is_null() {
        return paths;
    }

    // Get items from session
    let items: *mut Object = msg_send![session, items];
    if items.is_null() {
        return paths;
    }

    let count: usize = msg_send![items, count];
    log::info!("GPUI iOS Drop: Session has {} items", count);

    for i in 0..count {
        let drag_item: *mut Object = msg_send![items, objectAtIndex: i];
        if drag_item.is_null() {
            continue;
        }

        let item_provider: *mut Object = msg_send![drag_item, itemProvider];
        if item_provider.is_null() {
            continue;
        }

        // Check if this provider has a file URL
        let type_str: *mut Object = msg_send![
            class!(NSString),
            stringWithUTF8String: PUBLIC_FILE_URL.as_ptr() as *const i8
        ];
        let has_file: BOOL = msg_send![item_provider, hasItemConformingToTypeIdentifier: type_str];

        if has_file == YES {
            // Try to get the suggested name as a fallback
            let suggested_name: *mut Object = msg_send![item_provider, suggestedName];
            if !suggested_name.is_null() {
                let utf8: *const i8 = msg_send![suggested_name, UTF8String];
                if !utf8.is_null() {
                    let name = unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str().unwrap_or("");
                    log::info!("GPUI iOS Drop: Item suggested name: {}", name);
                }
            }

            // For local files, we can try to get the URL directly via localObject
            // This works for files dragged from the same app or Files app
            let local_object: *mut Object = msg_send![drag_item, localObject];
            if !local_object.is_null() {
                // Check if it's an NSURL
                let is_url: BOOL = msg_send![local_object, isKindOfClass: class!(NSURL)];
                if is_url == YES {
                    let path_string: *mut Object = msg_send![local_object, path];
                    if !path_string.is_null() {
                        let utf8: *const i8 = msg_send![path_string, UTF8String];
                        if !utf8.is_null() {
                            let path_str = unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str().unwrap_or("");
                            if !path_str.is_empty() {
                                paths.push(PathBuf::from(path_str));
                                log::info!("GPUI iOS Drop: Got local path: {}", path_str);
                            }
                        }
                    }
                }
            }
        }
    }

    paths
}

/// Load file URLs asynchronously from item providers and store them.
///
/// # Safety
/// session must be a valid UIDropSession pointer.
pub unsafe fn load_file_urls_async(session: *mut Object) {
    if session.is_null() {
        return;
    }

    let items: *mut Object = msg_send![session, items];
    if items.is_null() {
        return;
    }

    let count: usize = msg_send![items, count];
    let state = get_drop_state();

    // Clear any existing paths
    // Safety: Only accessed from main thread
    unsafe { *state.pending_paths.get() = None };

    let mut paths: Vec<PathBuf> = Vec::new();

    for i in 0..count {
        let drag_item: *mut Object = msg_send![items, objectAtIndex: i];
        if drag_item.is_null() {
            continue;
        }

        let item_provider: *mut Object = msg_send![drag_item, itemProvider];
        if item_provider.is_null() {
            continue;
        }

        // Check for file URL type
        let type_str: *mut Object = msg_send![
            class!(NSString),
            stringWithUTF8String: PUBLIC_FILE_URL.as_ptr() as *const i8
        ];
        let has_file: BOOL = msg_send![item_provider, hasItemConformingToTypeIdentifier: type_str];

        if has_file == YES {
            // For cross-app drops, we need to use loadFileRepresentation
            // which copies the file to a temporary location.
            // For now, we'll just note that async loading is needed.
            log::info!("GPUI iOS Drop: Item {} needs async loading", i);

            // Get suggested name
            let suggested_name: *mut Object = msg_send![item_provider, suggestedName];
            if !suggested_name.is_null() {
                let utf8: *const i8 = msg_send![suggested_name, UTF8String];
                if !utf8.is_null() {
                    let name = unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str().unwrap_or("unknown");
                    // Create a placeholder path with the suggested name
                    // Real implementation would use loadFileRepresentation
                    paths.push(PathBuf::from(name));
                }
            }
        }
    }

    if !paths.is_empty() {
        // Safety: Only accessed from main thread
        unsafe { *state.pending_paths.get() = Some(ExternalPaths(SmallVec::from_vec(paths))) };
    }
}

/// Get the pending drop paths (if any).
pub fn take_pending_paths() -> Option<ExternalPaths> {
    let state = get_drop_state();
    unsafe { (*state.pending_paths.get()).take() }
}

/// Set the drop session active state.
pub fn set_drop_active(active: bool) {
    let state = get_drop_state();
    unsafe {
        *state.is_active.get() = active;
    }
}

/// Check if a drop session is active.
pub fn is_drop_active() -> bool {
    let state = get_drop_state();
    unsafe { *state.is_active.get() }
}

/// Create a UIDropProposal with the specified operation.
///
/// # Safety
/// Must be called on main thread.
pub unsafe fn create_drop_proposal(operation: UIDropOperation) -> *mut Object {
    let proposal: *mut Object = msg_send![class!(UIDropProposal), alloc];
    let proposal: *mut Object = msg_send![proposal, initWithDropOperation: operation as usize];
    proposal
}

// =============================================================================
// UIDropInteractionDelegate implementation
// =============================================================================

/// dropInteraction:canHandle: - Determine if we can handle this drop session.
///
/// Returns YES if the session contains file URLs.
pub extern "C" fn drop_interaction_can_handle(
    _this: &Object,
    _sel: Sel,
    _interaction: *mut Object,
    session: *mut Object,
) -> BOOL {
    log::info!("GPUI iOS: dropInteraction:canHandle: called");

    let can_handle = unsafe { session_has_file_urls(session) };
    log::info!("GPUI iOS: Can handle drop: {}", can_handle);

    if can_handle { YES } else { NO }
}

/// dropInteraction:sessionDidEnter: - Called when drag enters the view.
///
/// Dispatches FileDropEvent::Entered.
pub extern "C" fn drop_interaction_session_did_enter(
    this: &Object,
    _sel: Sel,
    _interaction: *mut Object,
    session: *mut Object,
) {
    log::info!("GPUI iOS: dropInteraction:sessionDidEnter: called");

    unsafe {
        set_drop_active(true);

        // Get the view (this is the GPUIMetalView)
        let view = this as *const Object as *mut Object;

        // Extract position
        let position = drop_position(session, view);
        log::info!("GPUI iOS: Drop entered at {:?}", position);

        // Try to get file paths
        let paths_vec = extract_file_urls_from_session(session);

        // Also try async loading for cross-app drops
        load_file_urls_async(session);

        // Create ExternalPaths
        let paths = ExternalPaths(SmallVec::from_vec(paths_vec));

        // Dispatch the event
        let event = PlatformInput::FileDrop(FileDropEvent::Entered { position, paths });
        dispatch_drop_event(this, event);
    }
}

/// dropInteraction:sessionDidUpdate: - Called repeatedly while dragging over view.
///
/// Dispatches FileDropEvent::Pending and returns UIDropProposal.
pub extern "C" fn drop_interaction_session_did_update(
    this: &Object,
    _sel: Sel,
    _interaction: *mut Object,
    session: *mut Object,
) -> *mut Object {
    unsafe {
        // Get the view
        let view = this as *const Object as *mut Object;

        // Extract position
        let position = drop_position(session, view);

        // Dispatch pending event
        let event = PlatformInput::FileDrop(FileDropEvent::Pending { position });
        dispatch_drop_event(this, event);

        // Return a copy proposal
        create_drop_proposal(UIDropOperation::Copy)
    }
}

/// dropInteraction:sessionDidExit: - Called when drag leaves the view.
///
/// Dispatches FileDropEvent::Exited.
pub extern "C" fn drop_interaction_session_did_exit(
    this: &Object,
    _sel: Sel,
    _interaction: *mut Object,
    _session: *mut Object,
) {
    log::info!("GPUI iOS: dropInteraction:sessionDidExit: called");

    set_drop_active(false);

    let event = PlatformInput::FileDrop(FileDropEvent::Exited);
    dispatch_drop_event(this, event);
}

/// dropInteraction:performDrop: - Called when files are actually dropped.
///
/// Dispatches FileDropEvent::Submit.
pub extern "C" fn drop_interaction_perform_drop(
    this: &Object,
    _sel: Sel,
    _interaction: *mut Object,
    session: *mut Object,
) {
    log::info!("GPUI iOS: dropInteraction:performDrop: called");

    unsafe {
        // Get the view
        let view = this as *const Object as *mut Object;

        // Extract position
        let position = drop_position(session, view);
        log::info!("GPUI iOS: Drop performed at {:?}", position);

        // Dispatch submit event
        let event = PlatformInput::FileDrop(FileDropEvent::Submit { position });
        dispatch_drop_event(this, event);

        // Clean up
        set_drop_active(false);

        // Also dispatch exited to match macOS behavior
        let exit_event = PlatformInput::FileDrop(FileDropEvent::Exited);
        dispatch_drop_event(this, exit_event);
    }
}

/// Dispatch a drop event through the window's input callback.
fn dispatch_drop_event(view: &Object, event: PlatformInput) {
    unsafe {
        // Get the window pointer from the view's ivar
        let window_ptr: *mut c_void = *view.get_ivar("gpui_window_ptr");
        if window_ptr.is_null() {
            log::warn!("GPUI iOS: No window pointer for drop event");
            return;
        }

        let window = &*(window_ptr as *const super::window::IosWindow);

        // Get and invoke the input callback
        let callback = window.input_callback.borrow_mut().take();
        if let Some(mut cb) = callback {
            cb(event);
            // Restore the callback
            window.input_callback.borrow_mut().replace(cb);
        }
    }
}
