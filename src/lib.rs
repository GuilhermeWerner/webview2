//! Rust bindings for
//! [WebView2](https://docs.microsoft.com/en-us/microsoft-edge/hosting/webview2).
//!
//! The new Chromium based Edge browser (>= 82.0.448.0) need to be installed for
//! this to actually work. Or the
//! [`build`](struct.EnvironmentBuilder.html#method.build) method will return an
//! error.
//!
//! By default, this crate ships a copy of the `WebView2Loader.dll` file for the
//! target platform. At runtime, this dll will be loaded from memory with the
//! [memory-module-sys](https://crates.io/crates/memory-module-sys) library.
//! License of the DLL file (part of the WebView2 SDK) is included in the
//! `Microsoft.Web.WebView2.0.9.448` folder. You can also [use an external
//! `WebView2Loader.dll`
//! file](struct.EnvironmentBuilder.html#method.with_dll_file_path).
//!
//! There are high level, idiomatic Rust wrappers for most APIs. And there are
//! bindings to almost all the raw COM APIs in the `webview2-sys` crate. You can
//! use the `as_inner` methods to convert to raw COM objects and call all those
//! methods. The `callback` macro can be helpful for implementing callbacks as
//! COM objects.
#![cfg(windows)]
// Caused by the `com_interface` macro.
#![allow(clippy::cmp_null)]
#![allow(clippy::type_complexity)]

use com::{interfaces::IUnknown, ComInterface, ComPtr, ComRc};
#[cfg(feature = "memory-load-library")]
use memory_module_sys::{MemoryGetProcAddress, MemoryLoadLibrary};
#[cfg(feature = "memory-load-library")]
use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::fmt;
use std::io;
use std::mem::{self, MaybeUninit};
use std::path::Path;
use std::ptr;
use webview2_sys::*;
use widestring::{NulError, WideCStr, WideCString};
use winapi::shared::minwindef::*;
use winapi::shared::ntdef::*;
use winapi::shared::windef::*;
use winapi::shared::winerror::{
    E_FAIL, E_INVALIDARG, FACILITY_WIN32, HRESULT_CODE, HRESULT_FROM_WIN32, MAKE_HRESULT,
    SEVERITY_ERROR, SUCCEEDED, S_OK,
};
use winapi::um::combaseapi::{CoTaskMemAlloc, CoTaskMemFree};
use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};

#[cfg(all(feature = "memory-load-library", target_arch = "x86_64"))]
static WEBVIEW2_LOADER_DLL_CONTENT: &[u8] =
    include_bytes!("../Microsoft.Web.WebView2.0.9.488/build/x64/WebView2Loader.dll");
#[cfg(all(feature = "memory-load-library", target_arch = "x86"))]
static WEBVIEW2_LOADER_DLL_CONTENT: &[u8] =
    include_bytes!("../Microsoft.Web.WebView2.0.9.488/build/x86/WebView2Loader.dll");
#[cfg(all(feature = "memory-load-library", target_arch = "aarch64"))]
static WEBVIEW2_LOADER_DLL_CONTENT: &[u8] =
    include_bytes!("../Microsoft.Web.WebView2.0.9.488/build/arm64/WebView2Loader.dll");

#[cfg(feature = "memory-load-library")]
static WEBVIEW2_LOADER_LIBRARY: Lazy<std::result::Result<usize, i32>> = Lazy::new(|| unsafe {
    let library = MemoryLoadLibrary(
        WEBVIEW2_LOADER_DLL_CONTENT.as_ptr() as *const _,
        WEBVIEW2_LOADER_DLL_CONTENT.len(),
    ) as usize;
    if library == 0 {
        // io::Error is not copy or clone, so we use the raw os error.
        Err(io::Error::last_os_error().raw_os_error().unwrap())
    } else {
        Ok(library)
    }
});

/// Returns a pointer that implements the COM callback interface with the specified closure.
/// Inspired by C++ Microsoft::WRT::Callback.
#[macro_export]
macro_rules! callback {
    ($name:ident, move | $($arg:ident : $arg_type:ty),* $(,)?| -> $ret_type:ty { $($body:tt)* }) => {{
        #[com::co_class(implements($name))]
        struct Impl {
            cb: Box<dyn Fn($($arg_type),*) -> $ret_type>,
        }

        impl $name for Impl {
            unsafe fn invoke(&self, $($arg : $arg_type),*) -> $ret_type {
                (self.cb)($($arg),*)
            }
        }

        impl Impl {
            // It is never used.
            pub fn new() -> Box<Self> {
                unreachable!()
            }
            // Returns an owning ComPtr. Suitable for passing over FFI.
            // The receiver is responsible for releasing it.
            pub fn new_ptr(cb: impl Fn($($arg_type),*) -> $ret_type + 'static) -> com::ComPtr<dyn $name> {
                let e = Self::allocate(Box::new(cb));
                unsafe {
                    use com::interfaces::IUnknown;
                    e.add_ref();
                    com::ComPtr::<dyn $name>::new(Box::into_raw(e) as _)
                }
            }
        }

        Impl::new_ptr(move |$($arg : $arg_type),*| -> $ret_type { $($body)* })
    }}
}

// Call `AddRef` and convert to `ComRc`.
unsafe fn add_ref_to_rc<T: ComInterface + ?Sized>(
    ptr: *mut *mut <T as ComInterface>::VTable,
) -> ComRc<T> {
    let ptr = ComPtr::new(ptr);
    ptr.add_ref();
    ptr.upgrade()
}

include!("interfaces.rs");

#[com::co_class(implements(ICoreWebView2EnvironmentOptions))]
struct EnvironmentOptionsImpl {
    additional_browser_arguments: RefCell<Option<WideCString>>,
    language: RefCell<Option<WideCString>>,
    target_compatible_browser_version: RefCell<Option<WideCString>>,
}

impl EnvironmentOptionsImpl {
    fn new() -> Box<Self> {
        unreachable!()
    }

    fn from_builder(
        builder: &EnvironmentBuilder,
    ) -> Result<*mut *mut ICoreWebView2EnvironmentOptionsVTable> {
        let additional_browser_arguments = if let Some(v) = builder.additional_browser_arguments {
            Some(WideCString::from_str(v)?)
        } else {
            None
        };
        let language = if let Some(v) = builder.language {
            Some(WideCString::from_str(v)?)
        } else {
            None
        };
        let version = if let Some(v) = builder.target_compatible_browser_version {
            Some(WideCString::from_str(v)?)
        } else {
            None
        };
        let instance = Self::allocate(
            additional_browser_arguments.into(),
            language.into(),
            version.into(),
        );
        unsafe {
            instance.add_ref();
        }
        Ok(Box::into_raw(instance) as _)
    }
}

fn clone_wide_cstr_with_co_task_mem_alloc(s: &WideCStr) -> LPWSTR {
    let len = s.len() + 1;
    unsafe {
        let s1 = CoTaskMemAlloc(len * 2) as *mut u16;
        assert!(!s1.is_null());
        ptr::copy_nonoverlapping(s.as_ptr(), s1, len);
        s1
    }
}

impl ICoreWebView2EnvironmentOptions for EnvironmentOptionsImpl {
    unsafe fn get_additional_browser_arguments(
        &self,
        /* out, retval */ value: *mut LPWSTR,
    ) -> HRESULT {
        if let Some(v) = self.additional_browser_arguments.borrow().as_ref() {
            value.write(clone_wide_cstr_with_co_task_mem_alloc(&v));
        } else {
            value.write(ptr::null_mut());
        }
        S_OK
    }

    unsafe fn put_additional_browser_arguments(&self, /* in */ value: LPCWSTR) -> HRESULT {
        *self.additional_browser_arguments.borrow_mut() = Some(WideCString::from_ptr_str(value));
        S_OK
    }

    unsafe fn get_language(&self, /* out, retval */ value: *mut LPWSTR) -> HRESULT {
        if let Some(v) = self.language.borrow().as_ref() {
            value.write(clone_wide_cstr_with_co_task_mem_alloc(&v));
        } else {
            value.write(ptr::null_mut());
        }
        S_OK
    }

    unsafe fn put_language(&self, /* in */ value: LPCWSTR) -> HRESULT {
        *self.language.borrow_mut() = Some(WideCString::from_ptr_str(value));
        S_OK
    }

    unsafe fn get_target_compatible_browser_version(
        &self,
        /* out, retval */ value: *mut LPWSTR,
    ) -> HRESULT {
        if let Some(v) = self.target_compatible_browser_version.borrow().as_ref() {
            value.write(clone_wide_cstr_with_co_task_mem_alloc(&v));
        } else {
            value.write(ptr::null_mut());
        }
        S_OK
    }

    unsafe fn put_target_compatible_browser_version(
        &self,
        /* in */ value: LPCWSTR,
    ) -> HRESULT {
        *self.target_compatible_browser_version.borrow_mut() =
            Some(WideCString::from_ptr_str(value));
        S_OK
    }
}

/// A builder for calling the `CreateCoreWebView2EnvironmentWithDetails`
/// function.
#[derive(Default)]
pub struct EnvironmentBuilder<'a> {
    dll_file_path: Option<&'a Path>,
    browser_executable_folder: Option<&'a Path>,
    user_data_folder: Option<&'a Path>,
    additional_browser_arguments: Option<&'a str>,
    language: Option<&'a str>,
    target_compatible_browser_version: Option<&'a str>,
}

impl<'a> EnvironmentBuilder<'a> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_browser_executable_folder(self, browser_executable_folder: &'a Path) -> Self {
        Self {
            browser_executable_folder: Some(browser_executable_folder),
            ..self
        }
    }

    #[inline]
    pub fn with_user_data_folder(self, user_data_folder: &'a Path) -> Self {
        Self {
            user_data_folder: Some(user_data_folder),
            ..self
        }
    }

    #[inline]
    pub fn with_additional_browser_arguments(self, additional_browser_arguments: &'a str) -> Self {
        Self {
            additional_browser_arguments: Some(additional_browser_arguments),
            ..self
        }
    }

    #[inline]
    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    #[inline]
    pub fn with_target_compatible_browser_version(mut self, version: &'a str) -> Self {
        self.target_compatible_browser_version = Some(version);
        self
    }

    /// Set path to the `WebView2Loader.dll` file.
    ///
    /// We will use `LoadLibraryW` to load this DLL file instead of using the
    /// embedded DLL file.
    ///
    /// When the `memory-load-library` feature is not enabled, this method must
    /// have been called before calling `build`.
    #[inline]
    pub fn with_dll_file_path(self, dll_file_path: &'a Path) -> Self {
        Self {
            dll_file_path: Some(dll_file_path),
            ..self
        }
    }

    // Inline so that dead code elimination can eliminate the DLL file content
    // and the memory-module-sys functions when they are not used.
    #[inline]
    pub fn build(
        self,
        completed: impl FnOnce(Result<Environment>) -> Result<()> + 'static,
    ) -> Result<()> {
        let Self {
            dll_file_path,
            browser_executable_folder,
            user_data_folder,
            ..
        } = self;

        let create_fn: FnCreateCoreWebView2EnvironmentWithOptions = unsafe {
            if let Some(dll_file_path) = dll_file_path {
                let dll_file_path = WideCString::from_os_str(dll_file_path)?;
                let dll = LoadLibraryW(dll_file_path.as_ptr());
                if dll.is_null() {
                    return Err(io::Error::last_os_error().into());
                }
                let create_fn = GetProcAddress(
                    dll,
                    "CreateCoreWebView2EnvironmentWithOptions\0".as_ptr() as *const i8,
                );
                if create_fn.is_null() {
                    return Err(io::Error::last_os_error().into());
                }
                mem::transmute(create_fn)
            } else {
                #[cfg(feature = "memory-load-library")]
                {
                    let library =
                        (*WEBVIEW2_LOADER_LIBRARY).map_err(io::Error::from_raw_os_error)?;
                    let create_fn = MemoryGetProcAddress(
                        library as _,
                        "CreateCoreWebView2EnvironmentWithOptions\0".as_ptr() as *const i8,
                    );
                    if create_fn.is_null() {
                        return Err(io::Error::last_os_error().into());
                    }
                    mem::transmute(create_fn)
                }
                #[cfg(not(feature = "memory-load-library"))]
                panic!("webview2: DLL file path is not specified")
            }
        };

        let browser_executable_folder = if let Some(p) = browser_executable_folder {
            Some(WideCString::from_os_str(p)?)
        } else {
            None
        };
        let user_data_folder = if let Some(p) = user_data_folder {
            Some(WideCString::from_os_str(p)?)
        } else {
            None
        };
        let options = EnvironmentOptionsImpl::from_builder(&self)?;

        let completed = RefCell::new(Some(completed));
        let completed = callback!(
            ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler,
            move |result: HRESULT,
                  created_environment: *mut *mut ICoreWebView2EnvironmentVTable|
                  -> HRESULT {
                let result = check_hresult(result).map(move |_| Environment {
                    inner: unsafe { add_ref_to_rc(created_environment) },
                });
                to_hresult(completed.borrow_mut().take().unwrap()(result))
            }
        );

        check_hresult(unsafe {
            create_fn(
                browser_executable_folder
                    .as_ref()
                    .map(|p| p.as_ptr())
                    .unwrap_or(ptr::null()),
                user_data_folder
                    .as_ref()
                    .map(|p| p.as_ptr())
                    .unwrap_or(ptr::null()),
                options,
                completed.as_raw(),
            )
        })
    }
}

macro_rules! get {
    ($get_method:ident, $T: ident) => {
        pub fn $get_method(&self) -> Result<$T> {
            let mut value = MaybeUninit::<$T>::uninit();
            check_hresult(unsafe { self.inner.$get_method(value.as_mut_ptr()) })?;
            Ok(unsafe { value.assume_init() })
        }
    };
}

macro_rules! put {
    ($put_method:ident, $arg_name:ident : $T:ident) => {
        pub fn $put_method(&self, $arg_name: $T) -> Result<()> {
            check_hresult(unsafe { self.inner.$put_method($arg_name) })
        }
    };
}

macro_rules! get_interface {
    ($get_method:ident, $T: ident, $VT: ident) => {
        pub fn $get_method(&self) -> Result<$T> {
            let mut ppv = MaybeUninit::<*mut *mut $VT>::uninit();
            check_hresult(unsafe { self.inner.$get_method(ppv.as_mut_ptr()) })?;
            Ok(unsafe {
                $T {
                    inner: add_ref_to_rc(ppv.assume_init()),
                }
            })
        }
    };
}

macro_rules! put_interface {
    ($put_method:ident, $T: ident) => {
        pub fn $put_method(&self, i: $T) -> Result<()> {
            check_hresult(unsafe {
                // Convert to `ComPtr` so that it is not automatically released.
                self.inner.$put_method(ComPtr::from(i.inner).as_raw())
            })
        }
    };
}

macro_rules! get_bool {
    ($get_method:ident) => {
        pub fn $get_method(&self) -> Result<bool> {
            let mut enabled = MaybeUninit::<BOOL>::uninit();
            check_hresult(unsafe { self.inner.$get_method(enabled.as_mut_ptr()) })?;
            Ok(unsafe { enabled.assume_init() } != 0)
        }
    };
}

macro_rules! put_bool {
    ($put_method:ident) => {
        pub fn $put_method(&self, enabled: bool) -> Result<()> {
            let enabled = if enabled { 1 } else { 0 };
            check_hresult(unsafe { self.inner.$put_method(enabled) })
        }
    };
}

macro_rules! get_string {
    ($get_string_method:ident) => {
        pub fn $get_string_method(&self) -> Result<String> {
            let mut result: LPWSTR = ptr::null_mut();
            check_hresult(unsafe { self.inner.$get_string_method(&mut result) })?;
            let result1 = unsafe { WideCStr::from_ptr_str(result) };
            let result1 = result1.to_string().map_err(|_| Error { hresult: E_FAIL });
            unsafe {
                CoTaskMemFree(result as _);
            }
            result1
        }
    };
}

macro_rules! put_string {
    ($put_string_method:ident) => {
        pub fn $put_string_method(&self, message_string: &str) -> Result<()> {
            let message = WideCString::from_str(message_string)?;
            check_hresult(unsafe { self.inner.$put_string_method(message.as_ptr()) })
        }
    };
}

macro_rules! call {
    ($method:ident) => {
        pub fn $method(&self) -> Result<()> {
            check_hresult(unsafe { self.inner.$method() })
        }
    };
}

macro_rules! add_event_handler_controller {
    ($method:ident, $arg_type:ident) => {
        pub fn $method(
            &self,
            event_handler: impl Fn(Controller) -> Result<()> + 'static,
        ) -> Result<EventRegistrationToken> {
            let mut token = MaybeUninit::<EventRegistrationToken>::uninit();

            let event_handler = callback!(
                $arg_type,
                move |sender: *mut *mut ICoreWebView2ControllerVTable,
                      _args: *mut *mut com::interfaces::iunknown::IUnknownVTable|
                      -> HRESULT {
                    let sender = Controller {
                        inner: unsafe { add_ref_to_rc(sender) },
                    };
                    to_hresult(event_handler(sender))
                }
            );

            check_hresult(unsafe {
                self.inner
                    .$method(event_handler.as_raw(), token.as_mut_ptr())
            })?;
            Ok(unsafe { token.assume_init() })
        }
    };
}

macro_rules! add_event_handler_view {
    ($method:ident, $arg_type:ident) => {
        pub fn $method(
            &self,
            event_handler: impl Fn(WebView) -> Result<()> + 'static,
        ) -> Result<EventRegistrationToken> {
            let mut token = MaybeUninit::<EventRegistrationToken>::uninit();

            let event_handler = callback!(
                $arg_type,
                move |sender: *mut *mut ICoreWebView2VTable,
                      _args: *mut *mut com::interfaces::iunknown::IUnknownVTable|
                      -> HRESULT {
                    let sender = WebView {
                        inner: unsafe { add_ref_to_rc(sender) },
                    };
                    to_hresult(event_handler(sender))
                }
            );

            check_hresult(unsafe {
                self.inner
                    .$method(event_handler.as_raw(), token.as_mut_ptr())
            })?;
            Ok(unsafe { token.assume_init() })
        }
    };
}

macro_rules! add_event_handler {
    ($method:ident, $arg_type:ident, $arg_args:ident, $arg_args_type:ident) => {
        pub fn $method(
            &self,
            handler: impl Fn(WebView, $arg_args) -> Result<()> + 'static,
        ) -> Result<EventRegistrationToken> {
            let mut token = MaybeUninit::<EventRegistrationToken>::uninit();

            let handler = callback!($arg_type, move |sender: *mut *mut ICoreWebView2VTable,
                                                     args: *mut *mut $arg_args_type|
                  -> HRESULT {
                let sender = WebView {
                    inner: unsafe { add_ref_to_rc(sender) },
                };
                let args = $arg_args {
                    inner: unsafe { add_ref_to_rc(args) },
                };
                to_hresult(handler(sender, args))
            });

            check_hresult(unsafe { self.inner.$method(handler.as_raw(), token.as_mut_ptr()) })?;
            Ok(unsafe { token.assume_init() })
        }
    };
}

macro_rules! remove_event_handler {
    ($method:ident) => {
        pub fn $method(&self, token: EventRegistrationToken) -> Result<()> {
            check_hresult(unsafe { self.inner.$method(token) })
        }
    };
}

impl Environment {
    pub fn create_controller(
        &self,
        parent_window: HWND,
        completed: impl FnOnce(Result<Controller>) -> Result<()> + 'static,
    ) -> Result<()> {
        let completed = RefCell::new(Some(completed));
        let completed = callback!(
            ICoreWebView2CreateCoreWebView2ControllerCompletedHandler,
            move |result: HRESULT,
                  created_host: *mut *mut ICoreWebView2ControllerVTable|
                  -> HRESULT {
                let result = check_hresult(result).map(|_| Controller {
                    inner: unsafe { add_ref_to_rc(created_host) },
                });
                to_hresult(completed.borrow_mut().take().unwrap()(result))
            }
        );
        check_hresult(unsafe {
            self.inner
                .create_core_web_view2_controller(parent_window, completed.as_raw())
        })
    }
}

impl Controller {
    get_bool!(get_is_visible);
    put_bool!(put_is_visible);
    get!(get_bounds, RECT);
    put!(put_bounds, bounds: RECT);
    get!(get_zoom_factor, f64);
    put!(put_zoom_factor, zoom_factor: f64);
    add_event_handler_controller!(
        add_zoom_factor_changed,
        ICoreWebView2ZoomFactorChangedEventHandler
    );
    remove_event_handler!(remove_zoom_factor_changed);
    pub fn set_bounds_and_zoom_factor(&self, bounds: RECT, zoom_factor: f64) -> Result<()> {
        check_hresult(unsafe { self.inner.set_bounds_and_zoom_factor(bounds, zoom_factor) })
    }
    pub fn move_focus(&self, reason: MoveFocusReason) -> Result<()> {
        check_hresult(unsafe { self.inner.move_focus(reason) })
    }
    pub fn add_move_focus_requested(
        &self,
        handler: impl Fn(Controller, MoveFocusRequestedEventArgs) -> Result<()> + 'static,
    ) -> Result<EventRegistrationToken> {
        let mut token = MaybeUninit::<EventRegistrationToken>::uninit();

        let handler = callback!(
            ICoreWebView2MoveFocusRequestedEventHandler,
            move |sender: *mut *mut ICoreWebView2ControllerVTable,
                  args: *mut *mut ICoreWebView2MoveFocusRequestedEventArgsVTable|
                  -> HRESULT {
                let sender = Controller {
                    inner: unsafe { add_ref_to_rc(sender) },
                };
                let args = MoveFocusRequestedEventArgs {
                    inner: unsafe { add_ref_to_rc(args) },
                };
                to_hresult(handler(sender, args))
            }
        );

        check_hresult(unsafe {
            self.inner
                .add_move_focus_requested(handler.as_raw(), token.as_mut_ptr())
        })?;
        Ok(unsafe { token.assume_init() })
    }
    remove_event_handler!(remove_move_focus_requested);
    add_event_handler_controller!(add_got_focus, ICoreWebView2FocusChangedEventHandler);
    remove_event_handler!(remove_got_focus);
    add_event_handler_controller!(add_lost_focus, ICoreWebView2FocusChangedEventHandler);
    remove_event_handler!(remove_lost_focus);
    pub fn add_accelerator_key_pressed(
        &self,
        handler: impl Fn(Controller, AcceleratorKeyPressedEventArgs) -> Result<()> + 'static,
    ) -> Result<EventRegistrationToken> {
        let mut token = MaybeUninit::<EventRegistrationToken>::uninit();

        let handler = callback!(
            ICoreWebView2AcceleratorKeyPressedEventHandler,
            move |sender: *mut *mut ICoreWebView2ControllerVTable,
                  args: *mut *mut ICoreWebView2AcceleratorKeyPressedEventArgsVTable|
                  -> HRESULT {
                let sender = Controller {
                    inner: unsafe { add_ref_to_rc(sender) },
                };
                let args = AcceleratorKeyPressedEventArgs {
                    inner: unsafe { add_ref_to_rc(args) },
                };
                to_hresult(handler(sender, args))
            }
        );

        check_hresult(unsafe {
            self.inner
                .add_accelerator_key_pressed(handler.as_raw(), token.as_mut_ptr())
        })?;
        Ok(unsafe { token.assume_init() })
    }
    remove_event_handler!(remove_accelerator_key_pressed);
    get!(get_parent_window, HWND);
    put!(put_parent_window, top_level_window: HWND);
    call!(notify_parent_window_position_changed);
    call!(close);
    pub fn get_webview(&self) -> Result<WebView> {
        let mut ppv: *mut *mut ICoreWebView2VTable = ptr::null_mut();
        check_hresult(unsafe { self.inner.get_core_web_view2(&mut ppv) })?;
        Ok(WebView {
            inner: unsafe { add_ref_to_rc(ppv) },
        })
    }
}

impl WebView {
    pub fn get_settings(&self) -> Result<Settings> {
        let mut ppv: *mut *mut ICoreWebView2SettingsVTable = ptr::null_mut();
        check_hresult(unsafe { self.inner.get_settings(&mut ppv) })?;
        Ok(Settings {
            inner: unsafe { add_ref_to_rc(ppv) },
        })
    }
    get_string!(get_source);
    put_string!(navigate);
    put_string!(navigate_to_string);
    add_event_handler!(
        add_navigation_starting,
        ICoreWebView2NavigationStartingEventHandler,
        NavigationStartingEventArgs,
        ICoreWebView2NavigationStartingEventArgsVTable
    );
    remove_event_handler!(remove_navigation_starting);
    add_event_handler!(
        add_content_loading,
        ICoreWebView2ContentLoadingEventHandler,
        ContentLoadingEventArgs,
        ICoreWebView2ContentLoadingEventArgsVTable
    );
    remove_event_handler!(remove_content_loading);
    add_event_handler!(
        add_source_changed,
        ICoreWebView2SourceChangedEventHandler,
        SourceChangedEventArgs,
        ICoreWebView2SourceChangedEventArgsVTable
    );
    remove_event_handler!(remove_source_changed);
    add_event_handler_view!(add_history_changed, ICoreWebView2HistoryChangedEventHandler);
    remove_event_handler!(remove_history_changed);
    add_event_handler!(
        add_navigation_completed,
        ICoreWebView2NavigationCompletedEventHandler,
        NavigationCompletedEventArgs,
        ICoreWebView2NavigationCompletedEventArgsVTable
    );
    remove_event_handler!(remove_navigation_completed);
    add_event_handler!(
        add_frame_navigation_starting,
        ICoreWebView2NavigationStartingEventHandler,
        NavigationStartingEventArgs,
        ICoreWebView2NavigationStartingEventArgsVTable
    );
    remove_event_handler!(remove_frame_navigation_starting);
    add_event_handler!(
        add_script_dialog_opening,
        ICoreWebView2ScriptDialogOpeningEventHandler,
        ScriptDialogOpeningEventArgs,
        ICoreWebView2ScriptDialogOpeningEventArgsVTable
    );
    remove_event_handler!(remove_script_dialog_opening);
    add_event_handler!(
        add_permission_requested,
        ICoreWebView2PermissionRequestedEventHandler,
        PermissionRequestedEventArgs,
        ICoreWebView2PermissionRequestedEventArgsVTable
    );
    remove_event_handler!(remove_permission_requested);
    add_event_handler!(
        add_process_failed,
        ICoreWebView2ProcessFailedEventHandler,
        ProcessFailedEventArgs,
        ICoreWebView2ProcessFailedEventArgsVTable
    );
    remove_event_handler!(remove_process_failed);
    // Don't take an `Option<impl FnOnce>`:
    // https://users.rust-lang.org/t/solved-how-to-pass-none-to-a-function-when-an-option-closure-is-expected/10956/8
    pub fn add_script_to_execute_on_document_created(
        &self,
        script: &str,
        callback: impl FnOnce(String) -> Result<()> + 'static,
    ) -> Result<()> {
        let script = WideCString::from_str(script)?;
        let callback = RefCell::new(Some(callback));
        let callback = callback!(
            ICoreWebView2AddScriptToExecuteOnDocumentCreatedCompletedHandler,
            move |error_code: HRESULT, id: LPCWSTR| -> HRESULT {
                to_hresult(check_hresult(error_code).and_then(|_| {
                    let id = unsafe { WideCStr::from_ptr_str(id) }
                        .to_string()
                        .map_err(|_| Error::new(E_FAIL))?;
                    if let Some(callback) = callback.borrow_mut().take() {
                        callback(id)
                    } else {
                        Ok(())
                    }
                }))
            }
        );
        check_hresult(unsafe {
            self.inner
                .add_script_to_execute_on_document_created(script.as_ptr(), callback.as_raw())
        })
    }
    pub fn remove_script_to_execute_on_document_created(&self, id: &str) -> Result<()> {
        let id = WideCString::from_str(id)?;
        check_hresult(unsafe {
            self.inner
                .remove_script_to_execute_on_document_created(id.as_ptr())
        })
    }
    pub fn execute_script(
        &self,
        script: &str,
        callback: impl FnOnce(String) -> Result<()> + 'static,
    ) -> Result<()> {
        let script = WideCString::from_str(script)?;
        let callback = RefCell::new(Some(callback));
        let callback = callback!(
            ICoreWebView2ExecuteScriptCompletedHandler,
            move |error_code: HRESULT, result_object_as_json: LPCWSTR| -> HRESULT {
                to_hresult(check_hresult(error_code).and_then(|_| {
                    let result_object_as_json_string =
                        unsafe { WideCStr::from_ptr_str(result_object_as_json) }
                            .to_string()
                            .map_err(|_| Error::new(E_FAIL))?;
                    if let Some(callback) = callback.borrow_mut().take() {
                        callback(result_object_as_json_string)
                    } else {
                        Ok(())
                    }
                }))
            }
        );
        check_hresult(unsafe {
            self.inner
                .execute_script(script.as_ptr(), callback.as_raw())
        })
    }
    add_event_handler_view!(
        add_document_title_changed,
        ICoreWebView2DocumentTitleChangedEventHandler
    );
    remove_event_handler!(remove_document_title_changed);
    pub fn capture_preview(
        &self,
        image_format: CapturePreviewImageFormat,
        image_stream: Stream,
        handler: impl FnOnce(Result<()>) -> Result<()> + 'static,
    ) -> Result<()> {
        let handler = RefCell::new(Some(handler));
        let handler = callback!(
            ICoreWebView2CapturePreviewCompletedHandler,
            move |result: HRESULT| -> HRESULT {
                let handler = handler.borrow_mut().take().unwrap();
                to_hresult(handler(check_hresult(result)))
            }
        );
        let image_stream = ComPtr::from(image_stream.inner);

        check_hresult(unsafe {
            self.inner
                .capture_preview(image_format, image_stream.as_raw(), handler.as_raw())
        })
    }
    call!(reload);
    put_string!(post_web_message_as_json);
    put_string!(post_web_message_as_string);
    add_event_handler!(
        add_web_message_received,
        ICoreWebView2WebMessageReceivedEventHandler,
        WebMessageReceivedEventArgs,
        ICoreWebView2WebMessageReceivedEventArgsVTable
    );
    remove_event_handler!(remove_web_message_received);
    // TODO: call_dev_tools_protocol_method
    get!(get_browser_process_id, u32);
    get_bool!(get_can_go_back);
    get_bool!(get_can_go_forward);
    call!(go_back);
    call!(go_forward);
    // TODO: get_dev_tools_protocol_event_receiver
    call!(stop);
    add_event_handler!(
        add_new_window_requested,
        ICoreWebView2NewWindowRequestedEventHandler,
        NewWindowRequestedEventArgs,
        ICoreWebView2NewWindowRequestedEventArgsVTable
    );
    remove_event_handler!(remove_new_window_requested);
    get_string!(get_document_title);
    // TODO: add_host_object_to_script ??
    // TODO: remove_host_object_to_script ??
    call!(open_dev_tools_window);
    add_event_handler_view!(
        add_contains_full_screen_element_changed,
        ICoreWebView2ContainsFullScreenElementChangedEventHandler
    );
    remove_event_handler!(remove_contains_full_screen_element_changed);
    get_bool!(get_contains_full_screen_element);
    add_event_handler!(
        add_web_resource_requested,
        ICoreWebView2WebResourceRequestedEventHandler,
        WebResourceRequestedEventArgs,
        ICoreWebView2WebResourceRequestedEventArgsVTable
    );
    remove_event_handler!(remove_web_resource_requested);
    pub fn add_web_resource_requested_filter(
        &self,
        uri: &str,
        resource_context: WebResourceContext,
    ) -> Result<()> {
        let uri = WideCString::from_str(uri)?;
        check_hresult(unsafe {
            self.inner
                .add_web_resource_requested_filter(uri.as_ptr(), resource_context)
        })
    }
    pub fn remove_web_resource_requested_filter(
        &self,
        uri: &str,
        resource_context: WebResourceContext,
    ) -> Result<()> {
        let uri = WideCString::from_str(uri)?;
        check_hresult(unsafe {
            self.inner
                .remove_web_resource_requested_filter(uri.as_ptr(), resource_context)
        })
    }
    add_event_handler_view!(
        add_window_close_requested,
        ICoreWebView2WindowCloseRequestedEventHandler
    );
    remove_event_handler!(remove_window_close_requested);
}

impl Settings {
    get_bool!(get_is_script_enabled);
    put_bool!(put_is_script_enabled);

    get_bool!(get_is_web_message_enabled);
    put_bool!(put_is_web_message_enabled);

    get_bool!(get_are_default_script_dialogs_enabled);
    put_bool!(put_are_default_script_dialogs_enabled);

    get_bool!(get_is_status_bar_enabled);
    put_bool!(put_is_status_bar_enabled);

    get_bool!(get_are_dev_tools_enabled);
    put_bool!(put_are_dev_tools_enabled);

    get_bool!(get_are_default_context_menus_enabled);
    put_bool!(put_are_default_context_menus_enabled);

    get_bool!(get_are_remote_objects_allowed);
    put_bool!(put_are_remote_objects_allowed);

    get_bool!(get_is_zoom_control_enabled);
    put_bool!(put_is_zoom_control_enabled);

    get_bool!(get_is_built_in_error_page_enabled);
    put_bool!(put_is_built_in_error_page_enabled);
}

impl ContentLoadingEventArgs {
    get_bool!(get_is_error_page);
    get!(get_navigation_id, u64);
}

impl WebMessageReceivedEventArgs {
    get_string!(get_source);
    get_string!(try_get_web_message_as_string);
    get_string!(get_web_message_as_json);
}

impl HttpHeadersCollectionIterator {
    pub fn get_current_header(&self) -> Result<(String, String)> {
        let mut name = MaybeUninit::<LPWSTR>::uninit();
        let mut value = MaybeUninit::<LPWSTR>::uninit();
        unsafe {
            check_hresult(
                self.inner
                    .get_current_header(name.as_mut_ptr(), value.as_mut_ptr()),
            )?;
            let name = name.assume_init();
            let value = value.assume_init();
            let name1 = WideCStr::from_ptr_str(name)
                .to_string()
                .map_err(|_| Error::new(E_FAIL));
            let value1 = WideCStr::from_ptr_str(value)
                .to_string()
                .map_err(|_| Error::new(E_FAIL));

            CoTaskMemFree(name as _);
            CoTaskMemFree(value as _);

            Ok((name1?, value1?))
        }
    }
    get_bool!(get_has_current_header);
    get_bool!(move_next);
}

impl Iterator for HttpHeadersCollectionIterator {
    type Item = (String, String);

    fn next(&mut self) -> Option<(String, String)> {
        if self.get_has_current_header() != Ok(true) {
            return None;
        }
        let v = self.get_current_header().ok();
        let _ = self.move_next();
        v
    }
}

impl HttpRequestHeaders {
    pub fn get_header(&self, name: &str) -> Result<String> {
        let name = WideCString::from_str(name)?;
        let mut value = MaybeUninit::<LPWSTR>::uninit();
        unsafe {
            check_hresult(self.inner.get_header(name.as_ptr(), value.as_mut_ptr()))?;
            let value = value.assume_init();
            let value1 = WideCStr::from_ptr_str(value)
                .to_string()
                .map_err(|_| Error::new(E_FAIL));

            CoTaskMemFree(value as _);

            value1
        }
    }
    pub fn get_headers(&self, name: &str) -> Result<HttpHeadersCollectionIterator> {
        let name = WideCString::from_str(name)?;
        let mut iterator: *mut *mut ICoreWebView2HttpHeadersCollectionIteratorVTable =
            ptr::null_mut();
        check_hresult(unsafe { self.inner.get_headers(name.as_ptr(), &mut iterator) })?;
        Ok(HttpHeadersCollectionIterator {
            inner: unsafe { add_ref_to_rc(iterator) },
        })
    }
    pub fn contains(&self, name: &str) -> Result<bool> {
        let name = WideCString::from_str(name)?;
        let mut result = MaybeUninit::<BOOL>::uninit();
        check_hresult(unsafe { self.inner.contains(name.as_ptr(), result.as_mut_ptr()) })?;
        Ok(unsafe { result.assume_init() } != 0)
    }
    pub fn set_header(&self, name: &str, value: &str) -> Result<()> {
        let name = WideCString::from_str(name)?;
        let value = WideCString::from_str(value)?;
        check_hresult(unsafe { self.inner.set_header(name.as_ptr(), value.as_ptr()) })
    }
    put_string!(remove_header);
    get_interface!(
        get_iterator,
        HttpHeadersCollectionIterator,
        ICoreWebView2HttpHeadersCollectionIteratorVTable
    );
}

impl HttpResponseHeaders {
    pub fn get_header(&self, name: &str) -> Result<String> {
        let name = WideCString::from_str(name)?;
        let mut value = MaybeUninit::<LPWSTR>::uninit();
        unsafe {
            check_hresult(self.inner.get_header(name.as_ptr(), value.as_mut_ptr()))?;
            let value = value.assume_init();
            let value1 = WideCStr::from_ptr_str(value)
                .to_string()
                .map_err(|_| Error::new(E_FAIL));

            CoTaskMemFree(value as _);

            value1
        }
    }
    pub fn contains(&self, name: &str) -> Result<bool> {
        let name = WideCString::from_str(name)?;
        let mut result = MaybeUninit::<BOOL>::uninit();
        check_hresult(unsafe { self.inner.contains(name.as_ptr(), result.as_mut_ptr()) })?;
        Ok(unsafe { result.assume_init() } != 0)
    }
    pub fn append_header(&self, name: &str, value: &str) -> Result<()> {
        let name = WideCString::from_str(name)?;
        let value = WideCString::from_str(value)?;
        check_hresult(unsafe { self.inner.append_header(name.as_ptr(), value.as_ptr()) })
    }
    pub fn get_headers(&self, name: &str) -> Result<HttpHeadersCollectionIterator> {
        let name = WideCString::from_str(name)?;
        let mut iterator: *mut *mut ICoreWebView2HttpHeadersCollectionIteratorVTable =
            ptr::null_mut();
        check_hresult(unsafe { self.inner.get_headers(name.as_ptr(), &mut iterator) })?;
        Ok(HttpHeadersCollectionIterator {
            inner: unsafe { add_ref_to_rc(iterator) },
        })
    }
    get_interface!(
        get_iterator,
        HttpHeadersCollectionIterator,
        ICoreWebView2HttpHeadersCollectionIteratorVTable
    );
}

impl Deferral {
    call!(complete);
}

impl WebResourceRequest {
    get_string!(get_uri);
    put_string!(put_uri);
    get_string!(get_method);
    put_string!(put_method);
    get_interface!(get_content, Stream, IStreamVTable);
    put_interface!(put_content, Stream);
    get_interface!(
        get_headers,
        HttpRequestHeaders,
        ICoreWebView2HttpRequestHeadersVTable
    );
}

impl WebResourceResponse {
    get_interface!(get_content, Stream, IStreamVTable);
    put_interface!(put_content, Stream);
    get_interface!(
        get_headers,
        HttpResponseHeaders,
        ICoreWebView2HttpResponseHeadersVTable
    );
    get!(get_status_code, i32);
    put!(put_status_code, status_code: i32);
    get_string!(get_reason_phrase);
    put_string!(put_reason_phrase);
}

impl WebResourceRequestedEventArgs {
    get_interface!(
        get_request,
        WebResourceRequest,
        ICoreWebView2WebResourceRequestVTable
    );
    get_interface!(
        get_response,
        WebResourceResponse,
        ICoreWebView2WebResourceResponseVTable
    );
    put_interface!(put_response, WebResourceResponse);
    get_interface!(get_deferral, Deferral, ICoreWebView2DeferralVTable);
    get!(get_resource_context, WebResourceContext);
}

impl NavigationCompletedEventArgs {
    get_bool!(get_is_success);
    get!(get_web_error_status, WebErrorStatus);
    get!(get_navigation_id, u64);
}

impl NavigationStartingEventArgs {
    get_string!(get_uri);
    get_bool!(get_is_user_initiated);
    get_bool!(get_is_redirected);
    get_interface!(
        get_request_headers,
        HttpRequestHeaders,
        ICoreWebView2HttpRequestHeadersVTable
    );
    get_bool!(get_cancel);
    put_bool!(put_cancel);
    get!(get_navigation_id, u64);
}

impl SourceChangedEventArgs {
    get_bool!(get_is_new_document);
}

impl ScriptDialogOpeningEventArgs {
    get_string!(get_uri);
    get!(get_kind, ScriptDialogKind);
    get_string!(get_message);
    call!(accept);
    get_string!(get_default_text);
    get_string!(get_result_text);
    put_string!(put_result_text);
    get_interface!(get_deferral, Deferral, ICoreWebView2DeferralVTable);
}

impl PermissionRequestedEventArgs {
    get_string!(get_uri);
    get!(get_permission_kind, PermissionKind);
    get_bool!(get_is_user_initiated);
    get!(get_state, PermissionState);
    put!(put_state, state: PermissionState);
    get_interface!(get_deferral, Deferral, ICoreWebView2DeferralVTable);
}

impl ProcessFailedEventArgs {
    get!(get_process_failed_kind, ProcessFailedKind);
}

impl NewWindowRequestedEventArgs {
    get_string!(get_uri);
    put_interface!(put_new_window, WebView);
    get_interface!(get_new_window, WebView, ICoreWebView2VTable);
    put_bool!(put_handled);
    get_bool!(get_handled);
    get_bool!(get_is_user_initiated);
    get_interface!(get_deferral, Deferral, ICoreWebView2DeferralVTable);
}

impl MoveFocusRequestedEventArgs {
    get!(get_reason, MoveFocusReason);
    get_bool!(get_handled);
    put_bool!(put_handled);
}

impl AcceleratorKeyPressedEventArgs {
    get!(get_key_event_kind, KeyEventKind);
    get!(get_virtual_key, u32);
    get!(get_key_event_lparam, i32);
    get!(get_physical_key_status, PhysicalKeyStatus);
    get_bool!(get_handled);
    put_bool!(put_handled);
}

// Missing in winapi APIs. But present in its import libraries.
extern "stdcall" {
    fn SHCreateMemStream(p_init: *const u8, cb_init: UINT) -> *mut *mut IStreamVTable;
}

impl Stream {
    /// Create a stream from a byte buffer. (`SHCreateMemStream`)
    pub fn from_bytes(buf: &[u8]) -> Self {
        let ppv = unsafe { SHCreateMemStream(buf.as_ptr(), buf.len() as _) };
        assert!(!ppv.is_null());
        Self {
            // Do not need to add ref for this pointer.
            inner: unsafe { ComRc::from_raw(ppv) },
        }
    }

    /// Create a `Stream` from an owning raw pointer to an `IStream`.
    ///
    /// # Safety
    ///
    /// See `ComRc::from_raw`.
    pub unsafe fn from_raw(ppv: *mut *mut IStreamVTable) -> Self {
        Self {
            inner: ComRc::from_raw(ppv),
        }
    }
}

impl io::Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read_bytes = MaybeUninit::uninit();
        check_hresult(unsafe {
            self.inner.read(
                buf.as_mut_ptr() as *mut _,
                buf.len() as _,
                read_bytes.as_mut_ptr(),
            )
        })
        .map_err(|e| e.into_io_error())?;
        Ok(unsafe { read_bytes.assume_init() } as _)
    }
}

impl io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut written_bytes = MaybeUninit::uninit();
        check_hresult(unsafe {
            self.inner.write(
                buf.as_ptr() as *mut _,
                buf.len() as _,
                written_bytes.as_mut_ptr(),
            )
        })
        .map_err(|e| e.into_io_error())?;
        Ok(unsafe { written_bytes.assume_init() } as _)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Seek for Stream {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        use std::convert::TryInto;

        let (origin, amount) = match pos {
            io::SeekFrom::Start(x) => (/* STREAM_SEEK_SET */ 0, x.try_into().unwrap()),
            io::SeekFrom::Current(x) => (/* STREAM_SEEK_CUR */ 1, x),
            io::SeekFrom::End(x) => (/* STREAM_SEEK_END */ 2, x),
        };

        let mut new_pos = MaybeUninit::<u64>::uninit();

        check_hresult(unsafe {
            self.inner.seek(
                mem::transmute(amount),
                origin,
                new_pos.as_mut_ptr() as *mut _,
            )
        })
        .map_err(|e| e.into_io_error())?;
        Ok(unsafe { new_pos.assume_init() })
    }
}

#[doc(inline)]
pub use webview2_sys::{
    CapturePreviewImageFormat, EventRegistrationToken, KeyEventKind, MoveFocusReason,
    PermissionKind, PermissionState, PhysicalKeyStatus, ProcessFailedKind, ScriptDialogKind,
    WebErrorStatus, WebResourceContext,
};

/// WebView2 Error.
///
/// Actually it's just an `HRESULT`.
#[derive(Debug, Eq, PartialEq)]
pub struct Error {
    hresult: HRESULT,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "webview2 error, HRESULT {:#X}", self.hresult as u32)
    }
}

impl std::error::Error for Error {}

impl From<NulError<u16>> for Error {
    fn from(_: NulError<u16>) -> Error {
        Error {
            hresult: E_INVALIDARG,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        match e.raw_os_error() {
            Some(e) => Error::new(HRESULT_FROM_WIN32(e as u32)),
            _ => Error::new(E_FAIL),
        }
    }
}

impl Error {
    pub fn new(hresult: HRESULT) -> Self {
        Self { hresult }
    }

    fn into_io_error(self) -> io::Error {
        if (self.hresult & (0xffff_0000_u32 as i32))
            == MAKE_HRESULT(SEVERITY_ERROR, FACILITY_WIN32, 0)
        {
            io::Error::from_raw_os_error(HRESULT_CODE(self.hresult))
        } else {
            io::Error::new(io::ErrorKind::Other, self)
        }
    }

    pub fn hresult(&self) -> HRESULT {
        self.hresult
    }
}

/// Check a `HRESULT`, if it is `SUCCEEDED`, return `Ok(())`. Otherwide return
/// an error containing the `HRESULT`.
pub fn check_hresult(hresult: HRESULT) -> Result<()> {
    if SUCCEEDED(hresult) {
        Ok(())
    } else {
        Err(Error { hresult })
    }
}

fn to_hresult<T>(r: Result<T>) -> HRESULT {
    match r {
        Ok(_) => S_OK,
        Err(Error { hresult }) => hresult,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};

    #[test]
    fn test_stream() {
        let mut stream = Stream::from_bytes(b"hello,");
        stream.seek(io::SeekFrom::End(0)).unwrap();
        stream.write_all(b" world").unwrap();

        let mut buf = Vec::new();
        stream.seek(io::SeekFrom::Start(0)).unwrap();
        stream.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello, world");
    }
}
