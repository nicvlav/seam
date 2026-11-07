//! The edit controller half of the component: the parameters as the host
//! lists, reads and writes them.

use ::vst3::ComRef;
use ::vst3::Steinberg::Vst::{
    IComponentHandler, IEditControllerTrait, ParamID, ParamValue, ParameterInfo,
    ParameterInfo_::ParameterFlags_, String128, TChar, kRootUnitId,
};
use ::vst3::Steinberg::{
    FIDString, IBStream, IPlugView, int32, kInternalError, kInvalidArgument, kNotInitialized,
    kResultFalse, kResultOk, tresult,
};
use seam_params::Kind;

use super::Component;
use crate::Plugin;
use crate::engine::Engine;
use crate::vst3::text::{copy_utf16, read_utf16};

impl<P: Plugin> IEditControllerTrait for Component<P> {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        // The component's own state, which the store already holds.
        kResultOk
    }

    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        // The controller keeps no state of its own.
        kResultOk
    }

    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn getParameterCount(&self) -> int32 {
        let count = self.engine().map_or(0, |engine| engine.set().len());
        int32::try_from(count).unwrap_or(int32::MAX)
    }

    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        self.guard(kInternalError, || {
            // SAFETY: the SDK's contract: `info` is null or valid for
            // writing one `ParameterInfo` until this call returns.
            let Some(info) = (unsafe { info.as_mut() }) else {
                return kInvalidArgument;
            };
            let Some(set) = self.engine().map(Engine::set) else {
                return kNotInitialized;
            };
            let Some(id) = usize::try_from(index)
                .ok()
                .and_then(|index| set.id_at(index))
            else {
                return kInvalidArgument;
            };
            let decl = set.decl(id);
            info.id = set.host_id(id);
            copy_utf16(decl.name, &mut info.title);
            copy_utf16(decl.name, &mut info.shortTitle);
            copy_utf16(decl.unit, &mut info.units);
            // A stepped range stays continuous to the host.
            info.stepCount = match decl.kind {
                Kind::Bool | Kind::Choice(_) => decl.kind.steps().saturating_sub(1) as int32,
                Kind::Continuous(_) => 0,
            };
            info.defaultNormalizedValue = decl.kind.normalize(decl.default_value());
            info.unitId = kRootUnitId;
            let mut flags = ParameterFlags_::kCanAutomate;
            if matches!(decl.kind, Kind::Choice(_)) {
                flags |= ParameterFlags_::kIsList;
            }
            if decl.bypass {
                flags |= ParameterFlags_::kIsBypass;
            }
            info.flags = flags;
            kResultOk
        })
    }

    unsafe fn getParamStringByValue(
        &self,
        id: ParamID,
        normalized: ParamValue,
        string: *mut String128,
    ) -> tresult {
        self.guard(kInternalError, || {
            // SAFETY: the SDK's contract: `string` is null or valid for
            // writing one `String128` until this call returns.
            let (Some(string), Some((_, decl))) = (unsafe { string.as_mut() }, self.param(id))
            else {
                return kInvalidArgument;
            };
            copy_utf16(&decl.format_real(decl.host_value(normalized)), string);
            kResultOk
        })
    }

    unsafe fn getParamValueByString(
        &self,
        id: ParamID,
        string: *mut TChar,
        normalized: *mut ParamValue,
    ) -> tresult {
        self.guard(kInternalError, || {
            // SAFETY: the SDK's contract: `normalized` is null or valid for
            // writing one value until this call returns.
            let (Some(normalized), Some((param, decl))) =
                (unsafe { normalized.as_mut() }, self.param(id))
            else {
                return kInvalidArgument;
            };
            // SAFETY: the SDK's contract: `string` is null or a
            // NUL-terminated string of at most a `String128`, readable until
            // this call returns, as `read_utf16` requires.
            let Some(text) = (unsafe { read_utf16(string, 128) }) else {
                return kInvalidArgument;
            };
            let Some(engine) = self.engine() else {
                return kNotInitialized;
            };
            match decl.parse_real(&text, engine.set().value(param)) {
                Some(real) => {
                    *normalized = decl.kind.normalize(real);
                    kResultOk
                }
                None => kResultFalse,
            }
        })
    }

    unsafe fn normalizedParamToPlain(&self, id: ParamID, normalized: ParamValue) -> ParamValue {
        self.guard(normalized, || match self.param(id) {
            Some((_, decl)) => f64::from(decl.host_value(normalized)),
            None => normalized,
        })
    }

    unsafe fn plainParamToNormalized(&self, id: ParamID, plain: ParamValue) -> ParamValue {
        self.guard(0.0, || match self.param(id) {
            Some((_, decl)) => decl.kind.normalize(decl.restore_value(plain as f32)),
            None => 0.0,
        })
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        self.guard(0.0, || match (self.engine(), self.param(id)) {
            (Some(engine), Some((param, _))) => engine.set().normalized(param),
            _ => 0.0,
        })
    }

    unsafe fn setParamNormalized(&self, id: ParamID, normalized: ParamValue) -> tresult {
        self.guard(kInternalError, || {
            let (Some(engine), Some((param, _))) = (self.engine(), self.param(id)) else {
                return kInvalidArgument;
            };
            engine.controller_write(param, normalized);
            kResultOk
        })
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        self.guard(kInternalError, || {
            let Some(engine) = self.engine() else {
                return kNotInitialized;
            };
            let Some(main) = engine.main() else {
                return kResultFalse;
            };
            // SAFETY: the SDK's contract: `handler` is null or a live
            // `IComponentHandler` during this call; `to_com_ptr` adds a
            // reference, which keeps it alive after the call returns.
            let handler = unsafe { ComRef::from_raw(handler) }.map(|handler| handler.to_com_ptr());
            engine.connect(&main, handler);
            kResultOk
        })
    }

    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}
