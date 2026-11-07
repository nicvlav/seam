//! The factory a host gets from `GetPluginFactory`: one class, the
//! component.

use std::ffi::c_void;
use std::marker::PhantomData;

use ::vst3::Class;
use ::vst3::Steinberg::PClassInfo_::ClassCardinality_;
use ::vst3::Steinberg::PFactoryInfo_::FactoryFlags_;
use ::vst3::Steinberg::{
    FIDString, FUnknown, IPluginFactory2Trait, IPluginFactory3, IPluginFactory3Trait,
    IPluginFactoryTrait, PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo, TUID, int32,
    kInvalidArgument, kNoInterface, kResultOk, tresult,
};

use super::component::{class_id, create};
use super::text::{copy_utf8, copy_utf16};
use crate::Plugin;

/// The SDK version a class reports.
const SDK_VERSION: &str = "VST 3.8.0";
/// The category of an audio processor class.
const AUDIO_MODULE: &str = "Audio Module Class";

/// Lists and creates `P`.
pub(crate) struct Factory<P>(PhantomData<fn() -> P>);

impl<P: Plugin> Factory<P> {
    pub(crate) fn new() -> Self {
        Factory(PhantomData)
    }
}

impl<P: Plugin> Class for Factory<P> {
    type Interfaces = (IPluginFactory3,);
}

impl<P> std::fmt::Debug for Factory<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Factory").finish()
    }
}

/// The class's subcategories, joined by `|` ("Fx|EQ").
pub(crate) fn subcategories<P: Plugin>() -> String {
    P::INFO.vst3_categories().join("|")
}

/// Whether `index` names the one class.
fn is_ours(index: int32) -> bool {
    index == 0
}

impl<P: Plugin> IPluginFactoryTrait for Factory<P> {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        // SAFETY: the SDK's contract: `info` is null or valid for writing
        // one `PFactoryInfo` until this call returns.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return kInvalidArgument;
        };
        copy_utf8(P::INFO.vendor, &mut info.vendor);
        copy_utf8(P::INFO.url, &mut info.url);
        copy_utf8(P::INFO.email, &mut info.email);
        info.flags = FactoryFlags_::kUnicode as int32;
        kResultOk
    }

    unsafe fn countClasses(&self) -> int32 {
        1
    }

    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        // SAFETY: the SDK's contract: `info` is null or valid for writing one
        // of its type until this call returns.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return kInvalidArgument;
        };
        if !is_ours(index) {
            return kInvalidArgument;
        }
        info.cid = class_id::<P>();
        info.cardinality = ClassCardinality_::kManyInstances as int32;
        copy_utf8(AUDIO_MODULE, &mut info.category);
        copy_utf8(P::INFO.name, &mut info.name);
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        class: FIDString,
        interface: FIDString,
        object: *mut *mut c_void,
    ) -> tresult {
        if class.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the SDK's contract: a non-null `class` points to a 16-byte
        // class id, readable during this call.
        let class = unsafe { *class.cast::<TUID>() };
        if class != class_id::<P>() {
            if !object.is_null() {
                // SAFETY: the SDK's contract: a non-null `object` is valid
                // for writing one pointer.
                unsafe { *object = std::ptr::null_mut() };
            }
            return kNoInterface;
        }
        // SAFETY: the SDK's contract for `createInstance` is `create`'s:
        // `interface` null or a 16-byte id, `object` null or room for one
        // pointer.
        unsafe { create::<P>(interface, object) }
    }
}

impl<P: Plugin> IPluginFactory2Trait for Factory<P> {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        // SAFETY: the SDK's contract: `info` is null or valid for writing one
        // of its type until this call returns.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return kInvalidArgument;
        };
        if !is_ours(index) {
            return kInvalidArgument;
        }
        info.cid = class_id::<P>();
        info.cardinality = ClassCardinality_::kManyInstances as int32;
        copy_utf8(AUDIO_MODULE, &mut info.category);
        copy_utf8(P::INFO.name, &mut info.name);
        info.classFlags = 0;
        copy_utf8(&subcategories::<P>(), &mut info.subCategories);
        copy_utf8(P::INFO.vendor, &mut info.vendor);
        copy_utf8(P::INFO.version, &mut info.version);
        copy_utf8(SDK_VERSION, &mut info.sdkVersion);
        kResultOk
    }
}

impl<P: Plugin> IPluginFactory3Trait for Factory<P> {
    unsafe fn getClassInfoUnicode(&self, index: int32, info: *mut PClassInfoW) -> tresult {
        // SAFETY: the SDK's contract: `info` is null or valid for writing one
        // of its type until this call returns.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return kInvalidArgument;
        };
        if !is_ours(index) {
            return kInvalidArgument;
        }
        info.cid = class_id::<P>();
        info.cardinality = ClassCardinality_::kManyInstances as int32;
        copy_utf8(AUDIO_MODULE, &mut info.category);
        copy_utf16(P::INFO.name, &mut info.name);
        info.classFlags = 0;
        copy_utf8(&subcategories::<P>(), &mut info.subCategories);
        copy_utf16(P::INFO.vendor, &mut info.vendor);
        copy_utf16(P::INFO.version, &mut info.version);
        copy_utf16(SDK_VERSION, &mut info.sdkVersion);
        kResultOk
    }

    unsafe fn setHostContext(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
}
