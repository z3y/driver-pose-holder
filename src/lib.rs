#![allow(unsafe_op_in_unsafe_fn)]

use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

const PROVIDER_VERSION: &CStr = c"IServerTrackedDeviceProvider_004";
const HOST_VERSION: &CStr = c"IVRServerDriverHost_006";
const VR_INIT_ERROR_INTERFACE_NOT_FOUND: i32 = 105;
const VR_INIT_ERROR_NONE: i32 = 0;
const HOST_POSE_UPDATED_SLOT: usize = 1;

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
        install_hook(ctx);
        log(c"driver initialized.");
    }

    VR_INIT_ERROR_NONE
}

unsafe extern "C" fn provider_cleanup(_this: *mut DeviceProvider) {
    let slot = PATCHED_SLOT.swap(0, Ordering::AcqRel) as *mut usize;
    let orig = ORIGINAL.load(Ordering::Acquire);
    if !slot.is_null() && orig != 0 {
        write_slot(slot, orig);
    }
}

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

#[cfg(windows)]
unsafe fn write_slot(slot: *mut usize, value: usize) -> bool {
    unsafe extern "system" {
        fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old: *mut u32) -> i32;
    }
    const PAGE_READWRITE: u32 = 0x04;
    let mut old = 0u32;
    if VirtualProtect(
        slot as *mut c_void,
        size_of::<usize>(),
        PAGE_READWRITE,
        &mut old,
    ) == 0
    {
        return false;
    }
    ptr::write_volatile(slot, value);
    let mut ignored = 0u32;
    VirtualProtect(slot as *mut c_void, size_of::<usize>(), old, &mut ignored);
    true
}

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static PATCHED_SLOT: AtomicUsize = AtomicUsize::new(0);

unsafe fn install_hook(ctx: *mut DriverContext) {
    let mut error: i32 = VR_INIT_ERROR_NONE;
    let get_generic_interface = (*(*ctx).vtable).get_generic_interface;
    let host = (get_generic_interface)(ctx, HOST_VERSION.as_ptr(), &mut error);
    if error != VR_INIT_ERROR_NONE || host.is_null() {
        log(c"could not get generic interface host");
        return;
    }

    let vtable = *(host as *const *mut usize);
    let slot = vtable.add(HOST_POSE_UPDATED_SLOT);
    let current = ptr::read_volatile(slot);
    let ours = hook_pose_updated as *const () as usize;

    let msg = format!("host={:p} vtable={:p} slot={:#x}\0", host, vtable, current);
    let msg = CStr::from_bytes_with_nul(msg.as_bytes()).unwrap();
    log(msg);

    if current == ours {
        return;
    }

    ORIGINAL.store(current, Ordering::Release);

    if write_slot(slot, ours) {
        PATCHED_SLOT.store(slot as usize, Ordering::Release);
        log(c"hooked TrackedDevicePoseUpdated");
    } else {
        log(c"VirtualProtect failed to patch vtable");
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DriverPose {
    pose_time_offset: f64,
    q_world_from_driver_rotation: [f64; 4],
    vec_world_from_driver_translation: [f64; 3],
    q_driver_from_head_rotation: [f64; 4],
    vec_driver_from_head_translation: [f64; 3],
    vec_position: [f64; 3],
    vec_velocity: [f64; 3],
    vec_acceleration: [f64; 3],
    q_rotation: [f64; 4],
    vec_angular_velocity: [f64; 3],
    vec_angular_acceleration: [f64; 3],
    result: i32,
    pose_is_valid: bool,
    will_drift_in_yaw: bool,
    should_apply_head_model: bool,
    device_is_connected: bool,
}

static mut LAST_VALID_POSES: [Option<DriverPose>; 2] = [None; 2];
static mut VALID_FRAMES_COUNT: [usize; 2] = [0; 2];

type PoseUpdatedFn = unsafe extern "C" fn(*mut c_void, u32, *const DriverPose, u32);

unsafe extern "C" fn hook_pose_updated(
    this: *mut c_void,
    device: u32,
    pose: *const DriverPose,
    size: u32,
) {
    let orig: PoseUpdatedFn = std::mem::transmute(ORIGINAL.load(Ordering::Acquire));

    let mut copy = *pose;

    const MIN_VALID_FRAMES: usize = 5;

    if !pose.is_null() && device > 0 && device <= 2 && size as usize == size_of::<DriverPose>() {
        let index = (device - 1) as usize;

        let mut actually_valid = false;
        if copy.pose_is_valid {
            if VALID_FRAMES_COUNT[index] >= MIN_VALID_FRAMES {
                actually_valid = true;
            }
            VALID_FRAMES_COUNT[index] += 1;
        } else {
            VALID_FRAMES_COUNT[index] = 0;
        }

        if !(copy.pose_is_valid && actually_valid) {
            if let Some(last) = LAST_VALID_POSES[index] {
                copy = last;
                copy.pose_is_valid = true;
            }
        } else {
            LAST_VALID_POSES[index] = Some(copy);
        }
    }

    orig(this, device, &copy, size)
}
