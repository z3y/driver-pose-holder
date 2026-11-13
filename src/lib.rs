#![allow(unsafe_op_in_unsafe_fn)]

use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
};

const PROVIDER_VERSION: &CStr = c"IServerTrackedDeviceProvider_004";
const VR_INIT_ERROR_INTERFACE_NOT_FOUND: i32 = 105;
const VR_INIT_ERROR_NONE: i32 = 0;

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub unsafe extern "C" fn HmdDriverFactory(
    interface_name: *const c_char,
    return_code: *mut i32,
) -> *mut c_void {
    let interface_name = CStr::from_ptr(interface_name);

    if interface_name == PROVIDER_VERSION {
        return &PROVIDER as *const DeviceProvider as _;
    }

    if !return_code.is_null() {
        *return_code = VR_INIT_ERROR_INTERFACE_NOT_FOUND;
    }

    ptr::null_mut()
}

#[repr(C)]
struct DriverContext {
    vtable: *const DriverContextVtable,
}

#[repr(C)]
struct DriverContextVtable {
    get_generic_interface:
        unsafe extern "C" fn(*mut DriverContext, *const c_char, *mut i32) -> *mut c_void,
    get_driver_handle: unsafe extern "C" fn(*mut DriverContext) -> u64,
}

#[repr(C)]
struct DeviceProvider {
    vtable: *const DeviceProviderVtable,
}

unsafe impl Sync for DeviceProvider {}

#[repr(C)]
struct DeviceProviderVtable {
    init: unsafe extern "C" fn(*mut DeviceProvider, *mut DriverContext) -> i32,
    cleanup: unsafe extern "C" fn(*mut DeviceProvider),
    get_interface_versions: unsafe extern "C" fn(*mut DeviceProvider) -> *const *const c_char,
    run_frame: unsafe extern "C" fn(*mut DeviceProvider),
    should_block_standby_mode: unsafe extern "C" fn(*mut DeviceProvider) -> bool,
    enter_standby: unsafe extern "C" fn(*mut DeviceProvider),
    leave_standby: unsafe extern "C" fn(*mut DeviceProvider),
}

static PROVIDER_VTABLE: DeviceProviderVtable = DeviceProviderVtable {
    init: provider_init,
    cleanup: provider_cleanup,
    get_interface_versions: provider_get_interface_versions,
    run_frame: provider_run_frame,
    should_block_standby_mode: provider_should_block_standby_mode,
    enter_standby: provider_enter_standby,
    leave_standby: provider_leave_standby,
};

static PROVIDER: DeviceProvider = DeviceProvider {
    vtable: &PROVIDER_VTABLE,
};

#[repr(C)]
struct DriverLog {
    vtable: *const DriverLogVtable,
}

#[repr(C)]
struct DriverLogVtable {
    log: unsafe extern "C" fn(*mut DriverLog, *const c_char),
}

static mut LOGGER: *mut DriverLog = ptr::null_mut();

fn log(msg: &CStr) {
    unsafe {
        let logger = LOGGER;
        if logger.is_null() {
            return;
        }

        let log_fn = (*(*logger).vtable).log;

        log_fn(logger, msg.as_ptr() as _);
    }
}

unsafe extern "C" fn provider_init(_this: *mut DeviceProvider, ctx: *mut DriverContext) -> i32 {
    if !ctx.is_null() {
        let mut error: i32 = VR_INIT_ERROR_NONE;
        let log_ptr =
            ((*(*ctx).vtable).get_generic_interface)(ctx, c"IVRDriverLog_001".as_ptr(), &mut error);
        if error == VR_INIT_ERROR_NONE && !log_ptr.is_null() {
            LOGGER = log_ptr as *mut DriverLog;
        }
        log(c"driver initialized.");
    }

    VR_INIT_ERROR_NONE
}

unsafe extern "C" fn provider_cleanup(_this: *mut DeviceProvider) {}

static NO_VERSIONS: [usize; 1] = [0];

unsafe extern "C" fn provider_get_interface_versions(
    _this: *mut DeviceProvider,
) -> *const *const c_char {
    NO_VERSIONS.as_ptr().cast()
}

unsafe extern "C" fn provider_run_frame(_this: *mut DeviceProvider) {}
unsafe extern "C" fn provider_should_block_standby_mode(_this: *mut DeviceProvider) -> bool {
    false
}
unsafe extern "C" fn provider_enter_standby(_this: *mut DeviceProvider) {}
unsafe extern "C" fn provider_leave_standby(_this: *mut DeviceProvider) {}
