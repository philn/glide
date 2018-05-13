#![allow(non_camel_case_types)]
use core_foundation::base::TCFType;
use core_foundation::date::CFTimeInterval;
use core_foundation::string::{CFString, CFStringRef};
use std::ptr;

pub type kern_return_t = ::std::os::raw::c_int;
pub type IOReturn = kern_return_t;
pub type IOPMAssertionID = u32;

extern "C" {
    #[link(name = "IOKit", kind = "framework")]
    pub fn IOPMAssertionCreateWithDescription(
        AssertionType: CFStringRef,
        Name: CFStringRef,
        Details: CFStringRef,
        HumanReadableReason: CFStringRef,
        LocalizationBundlePath: CFStringRef,
        Timeout: CFTimeInterval,
        TimeoutAction: CFStringRef,
        AssertionID: *mut IOPMAssertionID,
    ) -> IOReturn;

    #[link(name = "IOKit", kind = "framework")]
    pub fn IOPMAssertionRelease(AssertionID: IOPMAssertionID) -> IOReturn;
}

pub fn prevent_display_sleep(reason: &str) -> u32 {
    let reason_cf = CFString::new(reason);
    let assertion_type = CFString::from_static_string("PreventUserIdleDisplaySleep");
    let mut assertion_id: IOPMAssertionID = 0;
    unsafe {
        IOPMAssertionCreateWithDescription(
            assertion_type.as_concrete_TypeRef(),
            reason_cf.as_concrete_TypeRef(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0 as f64,
            ptr::null(),
            &mut assertion_id,
        );
    }
    assertion_id as u32
}

pub fn release_sleep_assertion(assertion_id: u32) {
    unsafe {
        IOPMAssertionRelease(assertion_id as IOPMAssertionID);
    }
}
