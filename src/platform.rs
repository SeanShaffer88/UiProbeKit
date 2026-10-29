//! Platform layer: the only place Windows APIs are touched.
//! On non-Windows targets everything returns ERR_NOT_SUPPORTED
//! so the pure logic (spec parsing, tree serialization, find)
//! and the FFI contract stay testable anywhere — and so a host
//! on another platform gets a clean, honest error instead of a
//! fake empty result.
//!
//! Every extern block MUST carry #[link(name = "...")] naming
//! its import library (user32 / kernel32). Without it the build
//! fails at LINK time with LNK2019 — check and clippy cannot
//! catch it; CI's real build can.
//!
//! The COM runtime entry points (ole32) and the BSTR helpers
//! (oleaut32) are resolved dynamically with
//! LoadLibraryW/GetProcAddress, so this module adds NO import
//! library dependency beyond the house user32/kernel32 blocks,
//! and UI Automation itself is reached purely through
//! CoCreateInstance of CLSID_CUIAutomation — nothing links
//! against uiautomationcore.

use crate::{ActionSpec, TreeNode, WalkLimits, WindowInfo, ERR_NOT_SUPPORTED};

pub type PResult<T> = Result<T, (i32, String)>;

pub fn not_supported<T>(what: &str) -> PResult<T> {
    Err((
        ERR_NOT_SUPPORTED,
        format!("{what} is only available on Windows"),
    ))
}

/// Enumerate the top-level windows (Windows only).
#[cfg(windows)]
pub fn list_windows() -> PResult<Vec<WindowInfo>> {
    win::list_windows()
}

#[cfg(not(windows))]
pub fn list_windows() -> PResult<Vec<WindowInfo>> {
    not_supported("listing windows")
}

/// Build the control tree under `hwnd` from the live UI
/// Automation raw view, capped by `limits`. Returns the tree
/// and whether the walk hit a cap (Windows only).
#[cfg(windows)]
pub fn inspect_tree(hwnd: i64, limits: WalkLimits) -> PResult<(TreeNode, bool)> {
    uia::inspect_tree(hwnd, limits)
}

#[cfg(not(windows))]
pub fn inspect_tree(_hwnd: i64, _limits: WalkLimits) -> PResult<(TreeNode, bool)> {
    not_supported("inspecting a window's control tree")
}

/// Perform the spec's action on the element its path
/// addresses (Windows only).
#[cfg(windows)]
pub fn perform_action(spec: &ActionSpec) -> PResult<()> {
    uia::perform_action(spec)
}

#[cfg(not(windows))]
pub fn perform_action(_spec: &ActionSpec) -> PResult<()> {
    not_supported("performing UI actions")
}

// ---------------------------------------------------------------------------
// Top-level window enumeration (user32).
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use crate::{Rect, WindowInfo};
    use std::ffi::c_void;

    use super::PResult;
    use crate::ERR_PLATFORM;

    #[repr(C)]
    #[derive(Default)]
    struct RectRaw {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    type WndEnumProc = unsafe extern "system" fn(*mut c_void, isize) -> i32;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: WndEnumProc, lparam: isize) -> i32;
        fn GetWindowTextLengthW(hwnd: *mut c_void) -> i32;
        fn GetWindowTextW(hwnd: *mut c_void, text: *mut u16, max_count: i32) -> i32;
        fn GetClassNameW(hwnd: *mut c_void, class_name: *mut u16, max_count: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn GetWindowRect(hwnd: *mut c_void, rect: *mut RectRaw) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    }

    fn read_window(hwnd: *mut c_void) -> WindowInfo {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            let mut buf = vec![0u16; (len.max(0) as usize) + 1];
            let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            let title = String::from_utf16_lossy(&buf[..copied.max(0) as usize]);
            let mut class_buf = vec![0u16; 257];
            let copied = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
            let class_name = String::from_utf16_lossy(&class_buf[..copied.max(0) as usize]);
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut rr = RectRaw::default();
            let rect = if GetWindowRect(hwnd, &mut rr) != 0 {
                Rect::from_corners(rr.left, rr.top, rr.right, rr.bottom)
            } else {
                // A window whose rect cannot be read (e.g. it is
                // being destroyed) reports zeros rather than
                // failing the whole enumeration.
                Rect::default()
            };
            WindowInfo {
                hwnd: hwnd as usize as i64,
                title,
                class_name,
                pid,
                rect,
                visible: IsWindowVisible(hwnd) != 0,
            }
        }
    }

    unsafe extern "system" fn enum_callback(hwnd: *mut c_void, lparam: isize) -> i32 {
        let out = &mut *(lparam as *mut Vec<WindowInfo>);
        out.push(read_window(hwnd));
        1 // keep enumerating
    }

    pub fn list_windows() -> PResult<Vec<WindowInfo>> {
        let mut out: Vec<WindowInfo> = Vec::new();
        let ok = unsafe { EnumWindows(enum_callback, &mut out as *mut Vec<WindowInfo> as isize) };
        if ok == 0 {
            return Err((ERR_PLATFORM, "EnumWindows failed".to_string()));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// UI Automation over COM.
//
// Everything below was checked against the `windows` crate
// 0.62.2 generated bindings before it was written
// (src/Windows/Win32/UI/Accessibility/mod.rs of the crate) —
// vtable layouts are ABI, and a wrong slot calls the wrong
// method silently:
//
// - CLSID_CUIAutomation = FF48DBA4-60EF-4201-AA87-54103EEF594E
//   (crate const `CUIAutomation`).
// - IID_IUIAutomation = 30CBE57D-D9D0-452A-AB13-7AC5AC4825EE;
//   vtable after IUnknown: CompareElements, CompareRuntimeIds,
//   GetRootElement, ElementFromHandle (slot 6), ...,
//   RawViewWalker (slot 16).
// - IID_IUIAutomationTreeWalker =
//   4042C624-389C-4AFC-A630-9DF854A541FC; vtable:
//   GetParentElement (3), GetFirstChildElement (4),
//   GetLastChildElement (5), GetNextSiblingElement (6),
//   GetPreviousSiblingElement (7). Note Next precedes Previous.
// - IID_IUIAutomationElement =
//   D22108AA-8AC5-49A5-837B-37BBB3D7591E (NOT the
//   D6DD68D1-... value that circulates from memory); vtable:
//   SetFocus (3), GetCurrentPattern (16), CurrentControlType
//   (21), CurrentLocalizedControlType (22), CurrentName (23),
//   CurrentIsEnabled (28), CurrentAutomationId (29),
//   CurrentClassName (30), CurrentIsOffscreen (38),
//   CurrentBoundingRectangle (43).
// - Pattern IIDs: Invoke FB377FBE-8EA6-46D5-9C73-6499642D3059,
//   Toggle 94CF8058-9B8D-4AB9-8BFD-4CD0A33C8C70,
//   Value A94CD8B1-0844-4CD6-9D2D-640537AB39E9. Each pattern's
//   own method (Invoke / Toggle / SetValue) is slot 3.
// - Pattern ids (crate consts): UIA_InvokePatternId = 10000,
//   UIA_ValuePatternId = 10002, UIA_TogglePatternId = 10015.
//
// BSTR discipline: strings an element hands out are real
// BSTRs — read with SysStringLen, freed with SysFreeString.
// The string handed to ValuePattern::SetValue is allocated
// with SysAllocString: a bare NUL-terminated wide buffer is
// NOT a BSTR, and a provider that reads its length prefix
// would read garbage.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod uia {
    use super::PResult;
    use crate::{
        ActionKind, ActionSpec, NodeInfo, Rect, TreeNode, WalkLimits, WalkState, ERR_NOT_FOUND,
        ERR_NOT_SUPPORTED, ERR_PLATFORM,
    };
    use std::ffi::c_void;

    type HResult = i32;

    const S_OK: HResult = 0;
    const RPC_E_CHANGED_MODE: HResult = 0x8001_0106u32 as i32;
    const UIA_E_ELEMENTNOTAVAILABLE: HResult = 0x8004_0201u32 as i32;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;

    const UIA_INVOKE_PATTERN_ID: i32 = 10000;
    const UIA_VALUE_PATTERN_ID: i32 = 10002;
    const UIA_TOGGLE_PATTERN_ID: i32 = 10015;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const CLSID_CUI_AUTOMATION: Guid = Guid {
        data1: 0xff48_dba4,
        data2: 0x60ef,
        data3: 0x4201,
        data4: [0xaa, 0x87, 0x54, 0x10, 0x3e, 0xef, 0x59, 0x4e],
    };
    const IID_UI_AUTOMATION: Guid = Guid {
        data1: 0x30cb_e57d,
        data2: 0xd9d0,
        data3: 0x452a,
        data4: [0xab, 0x13, 0x7a, 0xc5, 0xac, 0x48, 0x25, 0xee],
    };
    const IID_INVOKE_PATTERN: Guid = Guid {
        data1: 0xfb37_7fbe,
        data2: 0x8ea6,
        data3: 0x46d5,
        data4: [0x9c, 0x73, 0x64, 0x99, 0x64, 0x2d, 0x30, 0x59],
    };
    const IID_TOGGLE_PATTERN: Guid = Guid {
        data1: 0x94cf_8058,
        data2: 0x9b8d,
        data3: 0x4ab9,
        data4: [0x8b, 0xfd, 0x4c, 0xd0, 0xa3, 0x3c, 0x8c, 0x70],
    };
    const IID_VALUE_PATTERN: Guid = Guid {
        data1: 0xa94c_d8b1,
        data2: 0x0844,
        data3: 0x4cd6,
        data4: [0x9d, 0x2d, 0x64, 0x05, 0x37, 0xab, 0x39, 0xe9],
    };

    // Vtable slots (IUnknown occupies 0..=2 everywhere). All
    // verified against the windows crate 0.62.2 vtbl structs
    // listed in the module comment above.
    const SLOT_QUERY_INTERFACE: usize = 0;
    const SLOT_RELEASE: usize = 2;
    // IUIAutomation
    const UIA_ELEMENT_FROM_HANDLE: usize = 6;
    const UIA_RAW_VIEW_WALKER: usize = 16;
    // IUIAutomationTreeWalker
    const TW_GET_FIRST_CHILD: usize = 4;
    const TW_GET_NEXT_SIBLING: usize = 6;
    // IUIAutomationElement
    const EL_SET_FOCUS: usize = 3;
    const EL_GET_CURRENT_PATTERN: usize = 16;
    const EL_CURRENT_CONTROL_TYPE: usize = 21;
    const EL_CURRENT_NAME: usize = 23;
    const EL_CURRENT_IS_ENABLED: usize = 28;
    const EL_CURRENT_AUTOMATION_ID: usize = 29;
    const EL_CURRENT_CLASS_NAME: usize = 30;
    const EL_CURRENT_IS_OFFSCREEN: usize = 38;
    const EL_CURRENT_BOUNDING_RECTANGLE: usize = 43;
    // Patterns: the action method is the first after IUnknown.
    const PATTERN_ACTION: usize = 3;

    #[repr(C)]
    #[derive(Default)]
    struct RectRaw {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    type CoInitializeExFn =
        unsafe extern "system" fn(reserved: *mut c_void, co_init: u32) -> HResult;
    type CoUninitializeFn = unsafe extern "system" fn();
    type CoCreateInstanceFn = unsafe extern "system" fn(
        rclsid: *const Guid,
        outer: *mut c_void,
        cls_context: u32,
        riid: *const Guid,
        result: *mut *mut c_void,
    ) -> HResult;
    type SysAllocStringFn = unsafe extern "system" fn(*const u16) -> *mut u16;
    type SysFreeStringFn = unsafe extern "system" fn(*mut u16);
    type SysStringLenFn = unsafe extern "system" fn(*mut u16) -> u32;

    struct Ole32 {
        co_initialize_ex: CoInitializeExFn,
        co_uninitialize: CoUninitializeFn,
        co_create_instance: CoCreateInstanceFn,
    }

    struct OleAut32 {
        sys_alloc_string: SysAllocStringFn,
        sys_free_string: SysFreeStringFn,
        sys_string_len: SysStringLenFn,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn hr_err(what: &str, hr: HResult) -> (i32, String) {
        (
            ERR_PLATFORM,
            format!("{what} failed: HRESULT 0x{:08X}", hr as u32),
        )
    }

    fn check(what: &str, hr: HResult) -> PResult<()> {
        if hr == S_OK {
            Ok(())
        } else {
            Err(hr_err(what, hr))
        }
    }

    fn load_module(dll: &str) -> PResult<*mut c_void> {
        let name = wide(dll);
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return Err((
                ERR_PLATFORM,
                format!("could not load {dll} to reach its entry points"),
            ));
        }
        // The module stays loaded for the process lifetime by
        // design: the resolved pointers must outlive every call
        // below, and these are system DLLs that are always safe
        // to keep mapped.
        Ok(module)
    }

    unsafe fn resolve<T>(module: *mut c_void, proc: &[u8], dll: &str) -> PResult<T> {
        let p = unsafe { GetProcAddress(module, proc.as_ptr()) };
        if p.is_null() {
            return Err((
                ERR_PLATFORM,
                format!(
                    "{dll} has no {} entry point",
                    String::from_utf8_lossy(&proc[..proc.len() - 1])
                ),
            ));
        }
        Ok(unsafe { std::mem::transmute_copy(&p) })
    }

    fn load_ole32() -> PResult<Ole32> {
        let module = load_module("ole32.dll")?;
        let co_initialize_ex: CoInitializeExFn =
            unsafe { resolve(module, b"CoInitializeEx\0", "ole32.dll") }?;
        let co_uninitialize: CoUninitializeFn =
            unsafe { resolve(module, b"CoUninitialize\0", "ole32.dll") }?;
        let co_create_instance: CoCreateInstanceFn =
            unsafe { resolve(module, b"CoCreateInstance\0", "ole32.dll") }?;
        Ok(Ole32 {
            co_initialize_ex,
            co_uninitialize,
            co_create_instance,
        })
    }

    fn load_oleaut32() -> PResult<OleAut32> {
        let module = load_module("oleaut32.dll")?;
        let sys_alloc_string: SysAllocStringFn =
            unsafe { resolve(module, b"SysAllocString\0", "oleaut32.dll") }?;
        let sys_free_string: SysFreeStringFn =
            unsafe { resolve(module, b"SysFreeString\0", "oleaut32.dll") }?;
        let sys_string_len: SysStringLenFn =
            unsafe { resolve(module, b"SysStringLen\0", "oleaut32.dll") }?;
        Ok(OleAut32 {
            sys_alloc_string,
            sys_free_string,
            sys_string_len,
        })
    }

    /// A COM apartment for one call. When the calling thread
    /// already has an apartment of a DIFFERENT model,
    /// CoInitializeEx reports RPC_E_CHANGED_MODE: COM calls
    /// still work in the existing apartment, so proceed — but
    /// that initialization is not ours, so it is not balanced
    /// with a CoUninitialize on drop.
    struct ComSession {
        ole: Ole32,
        uninit_owed: bool,
    }

    impl ComSession {
        fn new() -> PResult<ComSession> {
            let ole = load_ole32()?;
            let hr =
                unsafe { (ole.co_initialize_ex)(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
            if hr >= 0 {
                Ok(ComSession {
                    ole,
                    uninit_owed: true,
                })
            } else if hr == RPC_E_CHANGED_MODE {
                Ok(ComSession {
                    ole,
                    uninit_owed: false,
                })
            } else {
                Err(hr_err("CoInitializeEx", hr))
            }
        }

        fn create(&self, clsid: &Guid, iid: &Guid, what: &str) -> PResult<ComPtr> {
            let mut raw: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                (self.ole.co_create_instance)(
                    clsid,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    iid,
                    &mut raw,
                )
            };
            check(&format!("CoCreateInstance({what})"), hr)?;
            if raw.is_null() {
                return Err((
                    ERR_PLATFORM,
                    format!("CoCreateInstance({what}) returned a null interface"),
                ));
            }
            Ok(ComPtr(raw))
        }
    }

    impl Drop for ComSession {
        fn drop(&mut self) {
            if self.uninit_owed {
                unsafe { (self.ole.co_uninitialize)() }
            }
        }
    }

    /// An owned interface pointer: released through its vtable
    /// on drop.
    struct ComPtr(*mut c_void);

    impl Drop for ComPtr {
        fn drop(&mut self) {
            unsafe { release(self.0) }
        }
    }

    unsafe fn vtable(p: *mut c_void) -> *const *const c_void {
        unsafe { *(p as *const *const *const c_void) }
    }

    unsafe fn slot<T>(p: *mut c_void, index: usize) -> T {
        unsafe {
            let entry = *vtable(p).add(index);
            std::mem::transmute_copy(&entry)
        }
    }

    unsafe fn release(p: *mut c_void) {
        if !p.is_null() {
            type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
            let f: ReleaseFn = unsafe { slot(p, SLOT_RELEASE) };
            unsafe { f(p) };
        }
    }

    unsafe fn query_interface(p: *mut c_void, iid: &Guid, what: &str) -> PResult<ComPtr> {
        type QueryInterfaceFn =
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;
        let f: QueryInterfaceFn = unsafe { slot(p, SLOT_QUERY_INTERFACE) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { f(p, iid, &mut raw) };
        check(&format!("QueryInterface({what})"), hr)?;
        if raw.is_null() {
            return Err((
                ERR_PLATFORM,
                format!("QueryInterface({what}) returned a null interface"),
            ));
        }
        Ok(ComPtr(raw))
    }

    // -- IUIAutomation calls ------------------------------------------------

    unsafe fn element_from_handle(automation: *mut c_void, hwnd: i64) -> PResult<ComPtr> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HResult;
        let f: Fn = unsafe { slot(automation, UIA_ELEMENT_FROM_HANDLE) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { f(automation, hwnd as usize as *mut c_void, &mut raw) };
        if hr == UIA_E_ELEMENTNOTAVAILABLE {
            return Err((
                ERR_NOT_FOUND,
                format!("no UI Automation element for window handle {hwnd}"),
            ));
        }
        check("IUIAutomation::ElementFromHandle", hr)?;
        if raw.is_null() {
            return Err((
                ERR_NOT_FOUND,
                format!("no UI Automation element for window handle {hwnd}"),
            ));
        }
        Ok(ComPtr(raw))
    }

    unsafe fn raw_view_walker(automation: *mut c_void) -> PResult<ComPtr> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
        let f: Fn = unsafe { slot(automation, UIA_RAW_VIEW_WALKER) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { f(automation, &mut raw) };
        check("IUIAutomation::RawViewWalker", hr)?;
        if raw.is_null() {
            return Err((
                ERR_PLATFORM,
                "IUIAutomation::RawViewWalker returned a null walker".to_string(),
            ));
        }
        Ok(ComPtr(raw))
    }

    // -- IUIAutomationTreeWalker calls ---------------------------------------

    unsafe fn walker_child(
        walker: *mut c_void,
        slot_index: usize,
        what: &str,
        el: *mut c_void,
    ) -> PResult<Option<ComPtr>> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HResult;
        let f: Fn = unsafe { slot(walker, slot_index) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { f(walker, el, &mut raw) };
        check(what, hr)?;
        Ok(if raw.is_null() {
            None
        } else {
            Some(ComPtr(raw))
        })
    }

    // -- IUIAutomationElement property reads ----------------------------------
    //
    // A property read that fails (the element is being torn
    // down mid-walk, a provider is misbehaving) degrades to the
    // empty/zero value instead of failing the whole probe: the
    // node itself was real, and a probe that aborts on the
    // first stale element is useless on live desktops.

    unsafe fn bstr_prop(oleaut: &OleAut32, el: *mut c_void, slot_index: usize) -> Option<String> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HResult;
        let f: Fn = unsafe { slot(el, slot_index) };
        let mut p: *mut u16 = std::ptr::null_mut();
        let hr = unsafe { f(el, &mut p) };
        if hr < 0 {
            return None;
        }
        Some(bstr_to_string(oleaut, p))
    }

    unsafe fn bstr_to_string(oleaut: &OleAut32, p: *mut u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let len = unsafe { (oleaut.sys_string_len)(p) } as usize;
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) });
        unsafe { (oleaut.sys_free_string)(p) };
        s
    }

    unsafe fn i32_prop(el: *mut c_void, slot_index: usize) -> Option<i32> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
        let f: Fn = unsafe { slot(el, slot_index) };
        let mut v: i32 = 0;
        let hr = unsafe { f(el, &mut v) };
        if hr < 0 {
            None
        } else {
            Some(v)
        }
    }

    unsafe fn rect_prop(el: *mut c_void) -> Option<Rect> {
        type Fn = unsafe extern "system" fn(*mut c_void, *mut RectRaw) -> HResult;
        let f: Fn = unsafe { slot(el, EL_CURRENT_BOUNDING_RECTANGLE) };
        let mut rr = RectRaw::default();
        let hr = unsafe { f(el, &mut rr) };
        if hr < 0 {
            None
        } else {
            Some(Rect::from_corners(rr.left, rr.top, rr.right, rr.bottom))
        }
    }

    fn read_info(oleaut: &OleAut32, el: *mut c_void) -> NodeInfo {
        unsafe {
            NodeInfo {
                name: bstr_prop(oleaut, el, EL_CURRENT_NAME).unwrap_or_default(),
                control_type: i32_prop(el, EL_CURRENT_CONTROL_TYPE).unwrap_or(0),
                automation_id: bstr_prop(oleaut, el, EL_CURRENT_AUTOMATION_ID).unwrap_or_default(),
                class_name: bstr_prop(oleaut, el, EL_CURRENT_CLASS_NAME).unwrap_or_default(),
                rect: rect_prop(el).unwrap_or_default(),
                enabled: i32_prop(el, EL_CURRENT_IS_ENABLED).unwrap_or(0) != 0,
                offscreen: i32_prop(el, EL_CURRENT_IS_OFFSCREEN).unwrap_or(0) != 0,
            }
        }
    }

    // -- Tree building ---------------------------------------------------------

    fn build_node(
        oleaut: &OleAut32,
        walker: *mut c_void,
        el: *mut c_void,
        depth: usize,
        state: &mut WalkState,
    ) -> TreeNode {
        let info = read_info(oleaut, el);
        let mut node = TreeNode {
            info,
            children: Vec::new(),
        };
        if depth >= state.limits.max_depth {
            // At the depth cap: record whether anything was cut.
            let has_child =
                unsafe { walker_child(walker, TW_GET_FIRST_CHILD, "GetFirstChildElement", el) }
                    .ok()
                    .flatten()
                    .is_some();
            if has_child {
                state.cut();
            }
            return node;
        }
        let mut child =
            unsafe { walker_child(walker, TW_GET_FIRST_CHILD, "GetFirstChildElement", el) }
                .ok()
                .flatten();
        while let Some(c) = child {
            if !state.admit(depth + 1) {
                break;
            }
            let next =
                unsafe { walker_child(walker, TW_GET_NEXT_SIBLING, "GetNextSiblingElement", c.0) }
                    .ok()
                    .flatten();
            node.children
                .push(build_node(oleaut, walker, c.0, depth + 1, state));
            child = next;
        }
        node
    }

    pub fn inspect_tree(hwnd: i64, limits: WalkLimits) -> PResult<(TreeNode, bool)> {
        let session = ComSession::new()?;
        let oleaut = load_oleaut32()?;
        let automation =
            session.create(&CLSID_CUI_AUTOMATION, &IID_UI_AUTOMATION, "IUIAutomation")?;
        let root = unsafe { element_from_handle(automation.0, hwnd) }?;
        let walker = unsafe { raw_view_walker(automation.0) }?;
        let mut state = WalkState::new(limits);
        // The root is always admitted: inspect specs validate
        // max_nodes >= 1, and the find limits are generous.
        let _ = state.admit(0);
        let tree = build_node(&oleaut, walker.0, root.0, 0, &mut state);
        Ok((tree, state.truncated))
    }

    // -- Actions -----------------------------------------------------------------

    unsafe fn get_pattern(
        el: *mut c_void,
        pattern_id: i32,
        iid: &Guid,
        name: &str,
    ) -> PResult<ComPtr> {
        type Fn = unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HResult;
        let f: Fn = unsafe { slot(el, EL_GET_CURRENT_PATTERN) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { f(el, pattern_id, &mut raw) };
        check(
            &format!("IUIAutomationElement::GetCurrentPattern({name})"),
            hr,
        )?;
        if raw.is_null() {
            return Err((
                ERR_NOT_SUPPORTED,
                format!("the element does not support the {name} pattern"),
            ));
        }
        let unknown = ComPtr(raw);
        // The returned object is an IUnknown; the pattern
        // interface itself comes from QueryInterface. A QI
        // failure means the object is not really the pattern
        // (UIA's reserved not-supported placeholder behaves
        // this way) — report NOT_SUPPORTED, not a platform
        // error.
        match unsafe { query_interface(unknown.0, iid, name) } {
            Ok(p) => Ok(p),
            Err(_) => Err((
                ERR_NOT_SUPPORTED,
                format!("the element does not support the {name} pattern"),
            )),
        }
    }

    unsafe fn pattern_call(pattern: *mut c_void, what: &str) -> PResult<()> {
        type Fn = unsafe extern "system" fn(*mut c_void) -> HResult;
        let f: Fn = unsafe { slot(pattern, PATTERN_ACTION) };
        let hr = unsafe { f(pattern) };
        check(what, hr)
    }

    pub fn perform_action(spec: &ActionSpec) -> PResult<()> {
        let session = ComSession::new()?;
        let oleaut = load_oleaut32()?;
        let automation =
            session.create(&CLSID_CUI_AUTOMATION, &IID_UI_AUTOMATION, "IUIAutomation")?;
        let root = unsafe { element_from_handle(automation.0, spec.hwnd) }?;
        let walker = unsafe { raw_view_walker(automation.0) }?;

        // Resolve the path from the root element: each step is
        // "first child, then advance N siblings".
        let mut current = root;
        for (step, &index) in spec.path.iter().enumerate() {
            let mut child = unsafe {
                walker_child(
                    walker.0,
                    TW_GET_FIRST_CHILD,
                    "GetFirstChildElement",
                    current.0,
                )
            }?
            .ok_or_else(|| {
                (
                    ERR_NOT_FOUND,
                    format!("path step {step}: the element has no children"),
                )
            })?;
            for _ in 0..index {
                child = unsafe {
                    walker_child(
                        walker.0,
                        TW_GET_NEXT_SIBLING,
                        "GetNextSiblingElement",
                        child.0,
                    )
                }?
                .ok_or_else(|| {
                    (
                        ERR_NOT_FOUND,
                        format!("path step {step}: child index {index} is out of range"),
                    )
                })?;
            }
            current = child;
        }

        match spec.action {
            ActionKind::Focus => {
                type Fn = unsafe extern "system" fn(*mut c_void) -> HResult;
                let f: Fn = unsafe { slot(current.0, EL_SET_FOCUS) };
                let hr = unsafe { f(current.0) };
                check("IUIAutomationElement::SetFocus", hr)
            }
            ActionKind::Invoke => {
                let pattern = unsafe {
                    get_pattern(
                        current.0,
                        UIA_INVOKE_PATTERN_ID,
                        &IID_INVOKE_PATTERN,
                        "Invoke",
                    )
                }?;
                unsafe { pattern_call(pattern.0, "IUIAutomationInvokePattern::Invoke") }
            }
            ActionKind::Toggle => {
                let pattern = unsafe {
                    get_pattern(
                        current.0,
                        UIA_TOGGLE_PATTERN_ID,
                        &IID_TOGGLE_PATTERN,
                        "Toggle",
                    )
                }?;
                unsafe { pattern_call(pattern.0, "IUIAutomationTogglePattern::Toggle") }
            }
            ActionKind::SetValue => {
                let pattern = unsafe {
                    get_pattern(current.0, UIA_VALUE_PATTERN_ID, &IID_VALUE_PATTERN, "Value")
                }?;
                let value = spec.value.as_deref().unwrap_or("");
                let wide_value = wide(value);
                let bstr = unsafe { (oleaut.sys_alloc_string)(wide_value.as_ptr()) };
                if bstr.is_null() {
                    return Err((
                        ERR_PLATFORM,
                        "SysAllocString failed for the set_value string".to_string(),
                    ));
                }
                type Fn = unsafe extern "system" fn(*mut c_void, *mut u16) -> HResult;
                let f: Fn = unsafe { slot(pattern.0, PATTERN_ACTION) };
                let hr = unsafe { f(pattern.0, bstr) };
                unsafe { (oleaut.sys_free_string)(bstr) };
                check("IUIAutomationValuePattern::SetValue", hr)
            }
        }
    }
}
