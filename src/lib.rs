#![allow(unsafe_op_in_unsafe_fn)]

use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

// Controllers still snap back to an incorrect pose right after the pose reports to be valid
// This value can be increased if controllers pose keeps snapping
const MIN_VALID_FRAMES: usize = 5;

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

static LOGGER: AtomicUsize = AtomicUsize::new(0);
static PROPERTIES: AtomicUsize = AtomicUsize::new(0);

fn log_string(msg: &String) {
    let msg = CStr::from_bytes_with_nul(msg.as_bytes());
    match msg {
        Ok(msg) => log(msg),
        Err(_) => log(c"invalid string"),
    }
}

fn log(msg: &CStr) {
    unsafe {
        let logger = LOGGER.load(Ordering::Acquire) as *mut DriverLog;
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
        let logger =
            ((*(*ctx).vtable).get_generic_interface)(ctx, c"IVRDriverLog_001".as_ptr(), &mut error);
        if error == VR_INIT_ERROR_NONE && !logger.is_null() {
            LOGGER.store(logger as usize, Ordering::Release);
            log(c"IVRDriverLog_001 loaded");
        }

        let props = ((*(*ctx).vtable).get_generic_interface)(
            ctx,
            c"IVRProperties_001".as_ptr(),
            &mut error,
        );
        if error == VR_INIT_ERROR_NONE && !props.is_null() {
            PROPERTIES.store(props as usize, Ordering::Release);
            log(c"IVRProperties_001 loaded");
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

const MAX_DEVICES: usize = 64;

// should be thread safe, each device ID only ever touches its own index
static mut DEVICE_DATA: [DeviceData; MAX_DEVICES] = [DeviceData::EMPTY; MAX_DEVICES];
static mut DEVICE_KNOWN: [AtomicBool; MAX_DEVICES] =
    [const { AtomicBool::new(false) }; MAX_DEVICES];

static ACTIVE_HANDS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Eq)]
enum DeviceType {
    Controller,
    Hand,
    Other,
}

struct DeviceData {
    ty: DeviceType,
    valid_frames_count: usize,
    invalid_frames_count: usize,
    last_valid_pose: Option<DriverPose>,
    pose_holder_enabled_string: String,
    pose_holder_disabled_string: String,
    driver: *mut c_void,
}

impl DeviceData {
    const EMPTY: DeviceData = DeviceData {
        ty: DeviceType::Other,
        valid_frames_count: 0,
        invalid_frames_count: 0,
        last_valid_pose: None,
        pose_holder_enabled_string: String::new(),
        pose_holder_disabled_string: String::new(),
        driver: ptr::null_mut(),
    };
}

type PoseUpdatedFn = unsafe extern "C" fn(*mut c_void, u32, *const DriverPose, u32);

unsafe extern "C" fn hook_pose_updated(
    this: *mut c_void,
    device: u32,
    pose: *const DriverPose,
    size: u32,
) {
    let orig: PoseUpdatedFn = std::mem::transmute(ORIGINAL.load(Ordering::Acquire));

    let index = device as usize;

    if index >= MAX_DEVICES {
        return orig(this, device, pose, size);
    }

    let known = DEVICE_KNOWN[index].load(Ordering::Acquire);

    if !known
        && !pose.is_null()
        && size as usize == size_of::<DriverPose>()
        && (*pose).device_is_connected
        && (*pose).pose_is_valid
    {
        let info = fetch_info(device);
        if let Some(info) = info {
            const CONTROLLER_ID: i32 = 2;
            if info.class == CONTROLLER_ID {
                let ty = if info.is_hand {
                    DeviceType::Hand
                } else {
                    DeviceType::Controller
                };

                let pose_holder_enabled_string = format!("Enabled for {}\0", info.model);
                let pose_holder_disabled_string = format!("Disabled for {}\0", info.model);

                DEVICE_DATA[index] = DeviceData {
                    ty,
                    valid_frames_count: 0,
                    invalid_frames_count: 0,
                    last_valid_pose: None,
                    pose_holder_enabled_string,
                    pose_holder_disabled_string,
                    driver: this,
                };

                log_string(&format!("Found controller {:?}\0", info));
            } else {
                log_string(&format!("Skipping device {:?}\0", info));
            }

            DEVICE_KNOWN[index].store(true, Ordering::Release);
        }
    }

    if !pose.is_null()
        && device > 0
        && device < (MAX_DEVICES as u32)
        && size as usize == size_of::<DriverPose>()
        && known
    {
        let data = &mut DEVICE_DATA[index];

        let mut hand_tracking_active = false;

        if data.ty == DeviceType::Hand {
            if (*pose).device_is_connected && (*pose).pose_is_valid {
                if data.valid_frames_count == 0 {
                    ACTIVE_HANDS.fetch_add(1, Ordering::SeqCst);

                    for index in 1..MAX_DEVICES {
                        let data = &mut DEVICE_DATA[index];
                        if data.ty == DeviceType::Controller && !data.driver.is_null() {
                            match data.last_valid_pose {
                                Some(mut pose) => {
                                    if pose.pose_is_valid {
                                        pose.pose_is_valid = false;
                                        // force update pose to false for one frame because pose update doesnt run anymore
                                        orig(data.driver, index as u32, &pose, size);
                                    }
                                }
                                None => {}
                            }
                        }
                    }
                }

                data.valid_frames_count += 1;
                data.invalid_frames_count = 0;
            } else {
                if data.valid_frames_count > 0 {
                    ACTIVE_HANDS.fetch_sub(1, Ordering::SeqCst);
                }

                data.valid_frames_count = 0;
                data.invalid_frames_count += 1;
            }
        }

        if ACTIVE_HANDS.load(Ordering::Acquire) > 0 {
            hand_tracking_active = true;
        }

        if data.ty == DeviceType::Controller && !hand_tracking_active {
            let mut copy = *pose;
            let mut actually_valid = false;
            if copy.pose_is_valid {
                if data.valid_frames_count >= MIN_VALID_FRAMES {
                    actually_valid = true;
                }
                if data.valid_frames_count == 0 {}
                data.valid_frames_count += 1;
            } else {
                data.valid_frames_count = 0;
            }

            if !(copy.pose_is_valid && actually_valid) {
                if let Some(last) = data.last_valid_pose {
                    copy = last;
                    copy.pose_is_valid = true;
                    if data.invalid_frames_count == 0 {
                        log_string(&data.pose_holder_enabled_string);
                    }
                    data.invalid_frames_count += 1;
                }
            } else {
                data.invalid_frames_count = 0;
                data.last_valid_pose = Some(copy);
            }

            orig(this, device, &copy, size);
        } else {
            orig(this, device, pose, size);
        }
    } else {
        orig(this, device, pose, size);
    }
}

#[repr(C)]
struct Properties {
    vtable: *const PropertiesVtable,
}

#[repr(C)]
struct PropertiesVtable {
    read_property_batch: unsafe extern "C" fn(*mut Properties, u64, *mut PropertyRead, u32) -> i32,
    write_property_batch: usize,
    get_prop_error_name: usize,
    tracked_device_to_container: unsafe extern "C" fn(*mut Properties, u32) -> u64,
}

#[repr(C)]
struct PropertyRead {
    prop: u32,
    buffer: *mut c_void,
    buffer_size: u32,
    tag: u32,
    required_size: u32,
    error: i32,
}

unsafe fn read_raw(
    props: *mut Properties,
    container: u64,
    prop: u32,
    buf: &mut [u8],
) -> Result<usize, i32> {
    let mut r = PropertyRead {
        prop,
        buffer: buf.as_mut_ptr() as *mut c_void,
        buffer_size: buf.len() as u32,
        tag: 0,
        required_size: 0,
        error: 0,
    };
    ((*(*props).vtable).read_property_batch)(props, container, &mut r, 1);
    if r.error == 0 {
        Ok(r.required_size as usize)
    } else {
        Err(r.error)
    }
}

unsafe fn read_string(props: *mut Properties, container: u64, prop: u32) -> Result<String, i32> {
    let mut buf = [0u8; 256];
    let n = read_raw(props, container, prop, &mut buf)?;
    let n = n.min(buf.len());
    let end = buf[..n].iter().position(|&b| b == 0).unwrap_or(n);
    Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
}

unsafe fn read_i32(props: *mut Properties, container: u64, prop: u32) -> Result<i32, i32> {
    let mut buf = [0u8; 4];
    read_raw(props, container, prop, &mut buf)?;
    Ok(i32::from_ne_bytes(buf))
}

const PROP_MODEL_NUMBER: u32 = 1001;
const PROP_SERIAL_NUMBER: u32 = 1002;
const PROP_DEVICE_CLASS: u32 = 1029;
const PROP_CONTROLLER_TYPE: u32 = 7000;
const PROP_INPUT_PROFILE_PATH: u32 = 1037;

#[derive(Debug)]
#[allow(dead_code)]
struct DeviceInfo {
    pub class: i32,
    pub model: String,
    pub serial: String,
    pub controller_type: String,
    pub profile_path: String,
    pub is_hand: bool,
}

unsafe fn fetch_info(device: u32) -> Option<DeviceInfo> {
    let props = PROPERTIES.load(Ordering::Acquire) as *mut Properties;
    if props.is_null() {
        return None;
    }

    let container = ((*(*props).vtable).tracked_device_to_container)(props, device);

    let class = read_i32(props, container, PROP_DEVICE_CLASS).ok()?;

    log_string(&format!("Fetching info for device {}\0", device));

    let mut model = String::new();
    match read_string(props, container, PROP_MODEL_NUMBER) {
        Ok(value) => model = value,
        Err(_) => {}
    }

    let mut serial = String::new();
    match read_string(props, container, PROP_SERIAL_NUMBER) {
        Ok(value) => serial = value,
        Err(_) => {}
    }

    let mut controller_type = String::new();
    match read_string(props, container, PROP_CONTROLLER_TYPE) {
        Ok(value) => controller_type = value,
        Err(_) => {}
    }

    let mut profile_path = String::new();
    match read_string(props, container, PROP_INPUT_PROFILE_PATH) {
        Ok(value) => profile_path = value,
        Err(_) => {}
    }

    let is_hand = profile_path.to_lowercase().contains("hand")
        || controller_type.to_lowercase().contains("hand")
        || model.to_lowercase().contains("hand")
        || serial.to_lowercase().contains("hand");

    Some(DeviceInfo {
        class,
        model,
        serial,
        controller_type,
        profile_path,
        is_hand,
    })
}
