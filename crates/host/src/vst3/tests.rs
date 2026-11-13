//! The exporter through its COM interfaces, as a host drives it.

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::{c_char, c_void};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use ::vst3::Steinberg::PFactoryInfo_::FactoryFlags_;
use ::vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirection, BusDirections_, BusInfo,
    IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentHandlerTrait,
    IComponentTrait, IEditController, IEditControllerTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, IProcessContextRequirements,
    MediaType, MediaTypes_, ParamID, ParamValue, ProcessData, ProcessModes_,
    ProcessSetup as VstSetup, SpeakerArr, SpeakerArrangement, SymbolicSampleSizes_,
};
use ::vst3::Steinberg::{
    IBStream, IPluginBaseTrait, IPluginFactory, IPluginFactory2Trait, IPluginFactory3,
    IPluginFactory3Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo,
    int32, kInternalError, kInvalidArgument, kNoInterface, kNotImplemented, kNotInitialized,
    kResultFalse, kResultOk, tresult,
};
use ::vst3::{Class, ComPtr, ComWrapper, Interface};
use seam_params::{
    AudioParams, Declarer, Gesture, Kind, ParamDecl, ParamId, Params, ProgramParams, host_id,
};
use serde_json::{Map, Value as Json};

use super::component::Component;
use super::test_host::{Recorder, Stream};
use super::{HostCall, RESTART_LATENCY, RESTART_VALUES, Refused, TestHost, plugin_factory};
use crate::engine::timer::{detached, pump};
use crate::faults::{thread_faults, written};
use crate::{
    AuCodes, Block, Category, Feature, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
    Status, Tail,
};

const PARAMS: &[ParamDecl] = &[
    ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-24.0, 24.0, 0.1, None),
        0.0,
    )
    .with_unit("dB"),
    ParamDecl::new("mode", "Mode", Kind::Choice(&["A", "B", "C"]), 1.0),
    ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass(),
];

/// The test plugins' parameters, declared from [`PARAMS`].
struct Table;

impl Params for Table {
    fn declare(p: &mut Declarer) -> Self {
        p.extend(PARAMS.iter().copied());
        Table
    }
}

/// What the probe was asked to do and what it saw, shared by the plugin and
/// its processor, which a test host runs on the test's own thread.
#[derive(Default)]
struct ProbeState {
    idles: u32,
    deactivations: u32,
    drops: u32,
    resets: u32,
    panic_in_process: bool,
    panic_in_activate: bool,
    panic_in_idle: bool,
    edit: Option<(&'static str, f32)>,
    sync: Option<(&'static str, f32)>,
    hold: bool,
    latency: u32,
    flushed: Option<bool>,
    blocks: Vec<usize>,
    setups: Vec<ProcessSetup>,
    /// Processors dropped, wherever they drop.
    processor_drops: Arc<AtomicU32>,
    panic_in_processor_drop: bool,
}

thread_local! {
    static PROBE: RefCell<ProbeState> = RefCell::default();
}

fn probe<R>(f: impl FnOnce(&mut ProbeState) -> R) -> R {
    PROBE.with(|state| f(&mut state.borrow_mut()))
}

/// Keeps its parts, and ticks them.
struct Probe {
    parts: Parts,
    program: ProgramParams,
    held: Option<Gesture>,
}

impl Probe {
    fn id(&self, key: &str) -> ParamId {
        self.parts.set().id(key).expect("declared")
    }
}

struct ProbeProcessor {
    params: AudioParams,
    gain: ParamId,
    drops: Arc<AtomicU32>,
    panic_on_drop: bool,
}

impl Drop for ProbeProcessor {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic_on_drop, "a processor's drop's bug");
    }
}

impl Processor for ProbeProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        assert!(!probe(|state| state.panic_in_process), "a processor's bug");
        let tiny = std::hint::black_box(f32::MIN_POSITIVE);
        let product = std::hint::black_box(tiny * std::hint::black_box(0.5));
        probe(|state| {
            state.flushed = Some(product == 0.0);
            state.blocks.push(block.frames);
        });
        let add = self.params.value(self.gain);
        for channel in block.channels.iter_mut() {
            for sample in &mut channel[..block.frames] {
                *sample += add;
            }
        }
        Status::Continue
    }

    fn reset(&mut self) {
        probe(|state| state.resets += 1);
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        probe(|state| state.drops += 1);
    }
}

impl Plugin for Probe {
    const INFO: PluginInfo = PluginInfo {
        name: "Probe",
        vendor: "Seam",
        url: "https://seam.example",
        email: "probe@seam.example",
        description: "",
        version: "1.2.3",
        id: "dev.seam.probe",
        vst3_class_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        au_codes: AuCodes {
            subtype: *b"prob",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[Feature::Equalizer, Feature::Delay],
        layouts: &[Layout::STEREO, Layout::MONO],
        midi_input: false,
        midi_output: false,
    };
    type Params = Table;
    type Processor = ProbeProcessor;

    fn new(_: Table, parts: Parts) -> Self {
        // Each test's probe starts afresh, even on a thread another used.
        probe(|state| *state = ProbeState::default());
        Probe {
            program: parts.set().program(),
            parts,
            held: None,
        }
    }

    fn activate(&mut self, setup: &ProcessSetup) -> ProbeProcessor {
        assert!(
            !probe(|state| state.panic_in_activate),
            "an activation's bug"
        );
        probe(|state| state.setups.push(*setup));
        ProbeProcessor {
            params: self.parts.set().audio(),
            gain: self.id("gain"),
            drops: probe(|state| Arc::clone(&state.processor_drops)),
            panic_on_drop: probe(|state| state.panic_in_processor_drop),
        }
    }

    fn deactivate(&mut self, processor: ProbeProcessor) {
        probe(|state| state.deactivations += 1);
        drop(processor);
    }

    fn latency(&self) -> u32 {
        probe(|state| state.latency)
    }

    fn tail(&self) -> Tail {
        Tail::Frames(100)
    }

    fn idle(&mut self) {
        probe(|state| state.idles += 1);
        assert!(!probe(|state| state.panic_in_idle), "an idle call's bug");
        if let Some((key, value)) = probe(|state| state.edit.take()) {
            assert!(self.parts.ui().write(self.id(key), value));
        }
        if let Some((key, value)) = probe(|state| state.sync.take()) {
            self.program.sync(self.id(key), value);
        }
        if probe(|state| state.hold) && self.held.is_none() {
            let gesture = self.parts.ui().begin(self.id("gain"));
            gesture.write(3.0);
            self.held = Some(gesture);
        }
    }
}

/// The bytes a C string holds.
fn text_of(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    String::from_utf8(bytes).expect("UTF-8")
}

const GAIN: u32 = host_id("gain");
const MODE: u32 = host_id("mode");
const BYPASS: u32 = host_id("bypass");

/// A host with the probe set up and processing at 48 kHz, `max_frames` per
/// block.
fn running(max_frames: usize) -> TestHost<Probe> {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.setup(48_000.0, max_frames), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    host
}

#[test]
fn the_factory_lists_one_class_that_is_its_own_controller() {
    let faults = thread_faults();
    // SAFETY: the factory hands out one reference, which the pointer takes.
    let factory = unsafe { ComPtr::from_raw(plugin_factory::<Probe>().cast::<IPluginFactory>()) }
        .expect("a factory");
    let factory = factory
        .cast::<IPluginFactory3>()
        .expect("the third factory");
    // SAFETY: the factory is alive while `factory` holds its reference;
    // each info is a live, zeroed struct the call may write, `other` and
    // `class` are 16-byte ids, `object` is room for one pointer, and the
    // component it receives holds the reference `from_raw` takes over.
    unsafe {
        let mut info: PFactoryInfo = std::mem::zeroed();
        assert_eq!(factory.getFactoryInfo(&mut info), kResultOk);
        assert_eq!(text_of(&info.vendor), "Seam");
        assert_eq!(text_of(&info.url), "https://seam.example");
        assert_eq!(text_of(&info.email), "probe@seam.example");
        assert_eq!(info.flags, FactoryFlags_::kUnicode as int32);
        assert_eq!(factory.countClasses(), 1);

        let mut first: PClassInfo = std::mem::zeroed();
        assert_eq!(factory.getClassInfo(0, &mut first), kResultOk);
        assert_eq!(first.cid.map(|byte| byte as u8), Probe::INFO.vst3_class_id);
        assert_eq!(text_of(&first.category), "Audio Module Class");
        assert_eq!(text_of(&first.name), "Probe");
        assert_eq!(factory.getClassInfo(1, &mut first), kInvalidArgument);

        let mut class: PClassInfo2 = std::mem::zeroed();
        assert_eq!(factory.getClassInfo2(0, &mut class), kResultOk);
        assert_eq!(text_of(&class.name), "Probe");
        assert_eq!(text_of(&class.category), "Audio Module Class");
        assert_eq!(text_of(&class.subCategories), "Fx|EQ|Delay");
        assert_eq!(text_of(&class.version), "1.2.3");
        assert_eq!(text_of(&class.sdkVersion), "VST 3.8.0");
        assert_eq!(class.cid.map(|byte| byte as u8), Probe::INFO.vst3_class_id);
        assert_eq!(factory.getClassInfo2(1, &mut class), kInvalidArgument);
        let mut wide: PClassInfoW = std::mem::zeroed();
        assert_eq!(factory.getClassInfoUnicode(0, &mut wide), kResultOk);
        assert_eq!(String::from_utf16_lossy(&wide.name[..5]), "Probe");

        let mut object: *mut c_void = std::ptr::dangling_mut();
        let other = [0 as c_char; 16];
        let interface = IComponent::IID.as_ptr().cast::<c_char>();
        assert_eq!(
            factory.createInstance(other.as_ptr(), interface, &mut object),
            kNoInterface
        );
        assert!(object.is_null());
        let class = class.cid;
        assert_eq!(
            factory.createInstance(class.as_ptr(), interface, &mut object),
            kResultOk
        );
        let component = ComPtr::from_raw(object.cast::<IComponent>()).expect("a component");
        assert!(
            component.cast::<IEditController>().is_some(),
            "its own controller"
        );
        assert!(component.cast::<IAudioProcessor>().is_some());
        assert!(component.cast::<IProcessContextRequirements>().is_some());
        let mut controller = [0 as c_char; 16];
        assert_eq!(
            component.getControllerClassId(&mut controller),
            kNotImplemented
        );
    }
    assert_eq!(thread_faults(), faults);
}

#[test]
fn parameters_are_listed_in_declaration_order_by_their_key_hash() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.parameter_count(), 3);
    let gain = host.parameter(0).expect("gain");
    assert_eq!(
        (gain.id, gain.title.as_str(), gain.units.as_str()),
        (GAIN, "Gain", "dB")
    );
    assert_eq!(gain.step_count, 0, "a stepped range stays continuous");
    assert_eq!(gain.default_normalized, 0.5);
    assert!(gain.can_automate && !gain.is_list && !gain.is_bypass);
    let mode = host.parameter(1).expect("mode");
    assert_eq!((mode.id, mode.step_count, mode.is_list), (MODE, 2, true));
    assert_eq!(mode.default_normalized, 0.5);
    let bypass = host.parameter(2).expect("bypass");
    assert_eq!(
        (bypass.id, bypass.step_count, bypass.is_bypass),
        (BYPASS, 1, true)
    );
    assert_eq!(host.parameter(3), None);
}

#[test]
fn text_and_values_convert_through_the_declaration() {
    let host = TestHost::<Probe>::new();
    let decl = PARAMS[0];
    assert_eq!(
        host.text(GAIN, 0.75).as_deref(),
        Some(decl.format_real(12.0).as_str())
    );
    assert_eq!(host.text(MODE, 1.0).as_deref(), Some("C"));
    let parsed = host.parse(GAIN, "6").expect("a number");
    assert_eq!(parsed, decl.kind.normalize(6.0));
    assert_eq!(host.parse(GAIN, "loud"), None);
    assert_eq!(host.plain(GAIN, 1.0), 24.0);
    assert_eq!(host.normalize(GAIN, 12.0), 0.75);
    assert_eq!(host.plain(GAIN, 0.501), 0.0, "snapped to the step");
    assert_eq!(
        host.text(GAIN, 0.501).as_deref(),
        Some(decl.format_real(0.0).as_str()),
        "shown snapped too"
    );
    assert_eq!(host.text(7, 0.5), None, "an unknown id");
}

#[test]
fn the_host_writes_through_process_and_through_the_controller() {
    let mut host = running(64);
    let (mut left, mut right) = ([0.0f32; 8], [0.0f32; 8]);
    assert_eq!(
        host.process_in_place(
            &mut [&mut left, &mut right],
            8,
            &[(GAIN, 0.75), (GAIN, 0.25)]
        ),
        Ok(())
    );
    assert_eq!(host.normalized(GAIN), 0.75, "the first queue wins");
    assert_eq!(left, [12.0; 8], "the block hears it");
    assert_eq!(
        host.process_in_place(&mut [&mut left, &mut right], 0, &[(MODE, 1.0), (99, 1.0)]),
        Ok(())
    );
    assert_eq!(host.normalized(MODE), 1.0, "a flush of parameters");
    assert_eq!(probe(|state| state.blocks.len()), 1, "with no block");

    // While processing, a controller write is ignored: the host sends the
    // same value through `process`.
    assert_eq!(host.set_normalized(GAIN, 0.5), Ok(()));
    assert_eq!(host.normalized(GAIN), 0.75);
    assert_eq!(host.stop(), Ok(()));
    assert_eq!(host.set_normalized(GAIN, 0.5), Ok(()));
    assert_eq!(host.normalized(GAIN), 0.5);
    assert_eq!(
        host.set_normalized(99, 0.5),
        Err(Refused(kInvalidArgument)),
        "an unknown id"
    );
}

#[test]
fn layouts_follow_the_declaration() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.layout(), Layout::STEREO, "the first layout by default");
    assert_eq!(host.arrange(Layout::MONO), Ok(()));
    assert_eq!(host.layout(), Layout::MONO);
    assert_eq!(
        host.arrange(Layout::new(1, 2)),
        Err(Refused(kResultFalse)),
        "not a declared layout"
    );
    assert_eq!(host.arrange(Layout::new(6, 6)), Err(Refused(kResultFalse)));
    assert_eq!(host.layout(), Layout::MONO);
    let (plugin, processor, _) = host.interfaces();
    // SAFETY: the component is alive while the host holds its interfaces;
    // all zeros is a valid `BusInfo`, live through each call that writes it,
    // and `double` is a live `ProcessSetup` the call only reads.
    unsafe {
        let mut bus: BusInfo = std::mem::zeroed();
        let audio = MediaTypes_::kAudio as MediaType;
        let output = BusDirections_::kOutput as BusDirection;
        assert_eq!(plugin.getBusCount(audio, output), 1);
        assert_eq!(
            plugin.getBusCount(MediaTypes_::kEvent as MediaType, output),
            0
        );
        assert_eq!(plugin.getBusInfo(audio, output, 0, &mut bus), kResultOk);
        assert_eq!(bus.channelCount, 1);
        assert_eq!(
            plugin.getBusInfo(audio, output, 1, &mut bus),
            kInvalidArgument
        );
        assert_eq!(
            processor.canProcessSampleSize(SymbolicSampleSizes_::kSample64 as int32),
            kResultFalse
        );
        let mut double = VstSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample64 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 48_000.0,
        };
        assert_eq!(
            processor.setupProcessing(&mut double),
            kResultFalse,
            "f32 only"
        );
    }
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(
        host.arrange(Layout::STEREO),
        Err(Refused(kResultFalse)),
        "not while active"
    );
    assert_eq!(
        probe(|state| state.setups.last().map(|setup| setup.layout)),
        Some(Layout::MONO)
    );
}

#[test]
fn blocks_run_on_separate_or_shared_buffers_and_in_pieces() {
    let faults = thread_faults();
    let mut host = running(16);
    let (left, right) = ([1.0f32; 40], [2.0f32; 40]);
    let (mut out_left, mut out_right) = ([0.0f32; 40], [0.0f32; 40]);
    assert_eq!(
        host.process(
            &[&left, &right],
            &mut [&mut out_left, &mut out_right],
            40,
            &[(GAIN, 0.75)]
        ),
        Ok(())
    );
    assert_eq!(out_left, [13.0; 40]);
    assert_eq!(out_right, [14.0; 40]);
    assert_eq!(
        probe(|state| state.blocks.clone()),
        [16, 16, 8],
        "longer than the setup"
    );
    let (mut left, mut right) = ([1.0f32; 4], [2.0f32; 4]);
    assert_eq!(
        host.process_in_place(&mut [&mut left, &mut right], 4, &[]),
        Ok(())
    );
    assert_eq!((left, right), ([13.0; 4], [14.0; 4]));
    assert_eq!(
        probe(|state| state.flushed),
        Some(cfg!(all(
            not(miri),
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))),
        "denormals flush inside"
    );
    assert_eq!(thread_faults(), faults);
}

#[test]
fn an_inactive_or_panicking_processor_plays_silence() {
    let mut host = TestHost::<Probe>::new();
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    let mut left = [5.0f32; 8];
    assert_eq!(
        host.process_in_place(&mut [&mut left], 8, &[(GAIN, 1.0)]),
        Ok(())
    );
    assert_eq!(left, [0.0; 8], "not active");
    assert_eq!(host.silence_flags(), 0b1, "and flagged silent");
    assert_eq!(host.normalized(GAIN), 1.0, "yet the host's write landed");

    assert_eq!(host.activate(), Ok(()));
    let mut left = [5.0f32; 8];
    assert_eq!(host.process_in_place(&mut [&mut left], 8, &[]), Ok(()));
    assert_eq!(host.silence_flags(), 0, "playing");
    let faults = thread_faults();
    host.expect_faults(1);
    probe(|state| state.panic_in_process = true);
    let mut left = [5.0f32; 8];
    assert_eq!(host.process_in_place(&mut [&mut left], 8, &[]), Ok(()));
    assert_eq!(left, [0.0; 8]);
    assert_eq!(thread_faults(), faults + 1, "caught and recorded");
    probe(|state| state.panic_in_process = false);
    let mut left = [5.0f32; 8];
    assert_eq!(host.process_in_place(&mut [&mut left], 8, &[]), Ok(()));
    assert_eq!(left, [0.0; 8], "silent until the next activation");

    assert_eq!(host.deactivate(), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    let mut left = [5.0f32; 8];
    assert_eq!(host.process_in_place(&mut [&mut left], 8, &[]), Ok(()));
    assert_eq!(left, [29.0; 8], "playing again");
    assert_eq!(host.silence_flags(), 0);
    assert_eq!(thread_faults(), faults + 1);
}

#[test]
fn stopping_resets_before_the_next_block() {
    let mut host = running(64);
    let mut left = [0.0f32; 4];
    assert_eq!(host.process_in_place(&mut [&mut left], 4, &[]), Ok(()));
    assert_eq!(host.stop(), Ok(()));
    assert_eq!(probe(|state| state.resets), 0, "not on the stopping thread");
    assert_eq!(host.start(), Ok(()));
    assert_eq!(host.process_in_place(&mut [&mut left], 4, &[]), Ok(()));
    assert_eq!(probe(|state| state.resets), 1);
    assert_eq!(host.process_in_place(&mut [&mut left], 4, &[]), Ok(()));
    assert_eq!(probe(|state| state.resets), 1);
}

#[test]
fn activation_builds_the_processor_and_deactivation_hands_it_back() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.setup(96_000.0, 333), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.activate(), Ok(()), "twice rebuilds");
    assert_eq!(probe(|state| state.deactivations), 1, "the first went back");
    let setup = probe(|state| state.setups.clone());
    assert_eq!(setup.len(), 2);
    assert_eq!(
        (setup[0].sample_rate, setup[0].max_frames, setup[0].layout),
        (96_000.0, 333, Layout::STEREO)
    );
    assert_eq!((setup[0].max_events, setup[0].max_out_events), (0, 0));
    assert_eq!(host.deactivate(), Ok(()));
    assert_eq!(probe(|state| state.deactivations), 2);
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.tail(), 100);
    drop(host);
    assert_eq!(
        probe(|state| state.deactivations),
        3,
        "handed back when released active"
    );
}

#[test]
fn a_panicking_entry_point_answers_an_error() {
    let host = TestHost::<Probe>::new();
    host.expect_faults(1);
    let faults = thread_faults();
    probe(|state| state.panic_in_activate = true);
    assert_eq!(host.activate(), Err(Refused(kInternalError)));
    assert_eq!(thread_faults(), faults + 1);
    probe(|state| state.panic_in_activate = false);
    assert_eq!(host.activate(), Ok(()), "the instance still works");
}

#[test]
fn state_round_trips_through_streams_on_any_thread() {
    let mut host = running(64);
    let mut left = [0.0f32; 4];
    assert_eq!(
        host.process_in_place(&mut [&mut left], 4, &[(GAIN, 0.75), (MODE, 1.0)]),
        Ok(())
    );
    let saved = host.save().expect("a block");
    let json: serde_json::Value = serde_json::from_slice(&saved).expect("JSON");
    assert_eq!(json["parameters"]["gain"], 12.0);
    assert_eq!(json["parameters"]["mode"], 2);

    let (plugin, _, _) = host.interfaces();
    let plugin: ComPtr<IComponent> = plugin.clone();
    let block = br#"{"seamStateVersion": 1, "parameters": {"gain": -6}}"#;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let faults = thread_faults();
            let stream = ComWrapper::new(Stream::default());
            stream.bytes.borrow_mut().extend_from_slice(block);
            let pointer = stream.as_com_ref::<IBStream>().expect("a stream").as_ptr();
            // SAFETY: the component is alive while `plugin` holds its
            // reference, and `pointer` is a live `IBStream` until `stream`
            // drops, after both calls; save and load may run on any thread
            // but audio, which this is.
            unsafe {
                assert_eq!(plugin.setState(pointer), kResultOk);
                stream.bytes.borrow_mut().clear();
                stream.position.set(0);
                assert_eq!(plugin.getState(pointer), kResultOk);
            }
            let json: serde_json::Value =
                serde_json::from_slice(&stream.bytes.borrow()).expect("JSON");
            assert_eq!(json["parameters"]["gain"], -6.0);
            assert_eq!(
                json["parameters"]["mode"], 1,
                "a load assigns every parameter"
            );
            assert_eq!(thread_faults(), faults, "no main-thread state touched");
        });
    });
    assert_eq!(host.normalized(GAIN), PARAMS[0].kind.normalize(-6.0));
    assert_eq!(
        host.load(b"not a block"),
        Err(Refused(kResultFalse)),
        "the host is told it failed"
    );
    assert_eq!(
        host.load(b""),
        Err(Refused(kResultFalse)),
        "nor is nothing a block"
    );
    assert_eq!(host.normalized(GAIN), PARAMS[0].kind.normalize(-6.0));
}

#[test]
fn edits_syncs_and_latency_reach_the_host_after_a_tick() {
    let host = TestHost::<Probe>::new();
    host.tick();
    assert_eq!(host.calls(), []);
    probe(|state| state.edit = Some(("gain", 6.0)));
    host.tick();
    assert_eq!(
        host.calls(),
        [
            HostCall::Begin(GAIN),
            HostCall::Perform(GAIN, PARAMS[0].kind.normalize(6.0)),
            HostCall::End(GAIN),
        ]
    );
    probe(|state| state.sync = Some(("mode", 0.0)));
    host.tick();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_VALUES)]);
    probe(|state| state.latency = 64);
    assert_eq!(host.latency(), 0, "what the main thread last read");
    host.tick();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_LATENCY)]);
    assert_eq!(host.latency(), 64);
    assert_eq!(probe(|state| state.idles), 4);

    // A gesture still open when the host stops listening is ended for it.
    probe(|state| state.hold = true);
    host.tick();
    assert_eq!(
        host.calls(),
        [
            HostCall::Begin(GAIN),
            HostCall::Perform(GAIN, PARAMS[0].kind.normalize(3.0))
        ]
    );
    let (plugin, _, _) = host.interfaces();
    // SAFETY: the component is alive while the host holds its interfaces,
    // the call passes no pointer, and this is its main thread.
    assert_eq!(unsafe { plugin.terminate() }, kResultOk);
    assert_eq!(host.calls(), [HostCall::End(GAIN)]);
}

#[test]
fn a_main_thread_call_from_another_thread_is_refused() {
    let host = TestHost::<Probe>::new();
    // On the other thread; the host's check sees every thread's.
    host.expect_faults(1);
    let (plugin, _, controller) = host.interfaces();
    let (plugin, controller): (ComPtr<IComponent>, ComPtr<IEditController>) =
        (plugin.clone(), controller.clone());
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let faults = thread_faults();
            // SAFETY: the component is alive while `plugin` and
            // `controller` hold their references, and no call passes a
            // pointer; from this thread, which is not its main one, it must
            // refuse the main-thread call without touching that state.
            unsafe {
                assert_eq!(plugin.setActive(1), kResultFalse);
                assert_eq!(thread_faults(), faults + 1);
                assert_eq!(controller.getParameterCount(), 3, "no main-thread state");
                assert_eq!(controller.getParamNormalized(GAIN), 0.5);
            }
            assert_eq!(thread_faults(), faults + 1);
        });
    });
    assert_eq!(host.activate(), Ok(()), "on its own thread");
}

/// A plugin with one layout, whose processor adds 1 to every channel.
struct Shaped<const INPUTS: usize, const OUTPUTS: usize>;

struct ShapedProcessor;

impl Processor for ShapedProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        for channel in block.channels.iter_mut() {
            for sample in &mut channel[..block.frames] {
                *sample += 1.0;
            }
        }
        Status::Continue
    }
}

impl<const INPUTS: usize, const OUTPUTS: usize> Plugin for Shaped<INPUTS, OUTPUTS> {
    const INFO: PluginInfo = PluginInfo {
        name: "Shaped",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "1.0.0",
        id: "dev.seam.shaped",
        vst3_class_id: [2; 16],
        au_codes: AuCodes {
            subtype: *b"shap",
            manufacturer: *b"Seam",
        },
        category: if INPUTS == 0 {
            Category::Instrument
        } else {
            Category::Effect
        },
        features: &[],
        layouts: &[Layout::new(INPUTS, OUTPUTS)],
        midi_input: false,
        midi_output: false,
    };
    type Params = Table;
    type Processor = ShapedProcessor;

    fn new(_: Table, _: Parts) -> Self {
        Shaped
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> ShapedProcessor {
        ShapedProcessor
    }
}

/// A host processing `P` at 48 kHz, up to 64 frames a block.
fn playing<P: Plugin>() -> TestHost<P> {
    let host = TestHost::<P>::new();
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    host
}

const AUDIO: MediaType = MediaTypes_::kAudio as MediaType;
const INPUT: BusDirection = BusDirections_::kInput as BusDirection;
const OUTPUT: BusDirection = BusDirections_::kOutput as BusDirection;

#[test]
fn a_plugin_without_input_has_no_input_bus() {
    let idle = TestHost::<Shaped<0, 2>>::new();
    assert_eq!(
        idle.arrange(Layout::new(0, 2)),
        Ok(()),
        "arranged with no input bus"
    );
    let mut host = playing::<Shaped<0, 2>>();
    let (plugin, _, _) = host.interfaces();
    // SAFETY: the component is alive while the host holds its interfaces,
    // and the calls pass no pointer.
    unsafe {
        assert_eq!(plugin.getBusCount(AUDIO, INPUT), 0);
        assert_eq!(plugin.getBusCount(AUDIO, OUTPUT), 1);
    }
    assert_eq!(host.layout(), Layout::new(0, 2));
    let (mut left, mut right) = ([5.0f32; 4], [5.0f32; 4]);
    assert_eq!(
        host.process(&[], &mut [&mut left, &mut right], 4, &[]),
        Ok(())
    );
    assert_eq!(
        (left, right),
        ([1.0; 4], [1.0; 4]),
        "the outputs start zeroed"
    );
}

#[test]
fn mono_in_stereo_out_starts_the_second_channel_silent() {
    let mut host = playing::<Shaped<1, 2>>();
    let (plugin, _, _) = host.interfaces();
    // SAFETY: the component is alive while the host holds its interfaces,
    // and all zeros is a valid `BusInfo`, live through each call that writes
    // it.
    unsafe {
        let mut bus: BusInfo = std::mem::zeroed();
        assert_eq!(plugin.getBusInfo(AUDIO, INPUT, 0, &mut bus), kResultOk);
        assert_eq!(bus.channelCount, 1);
        assert_eq!(plugin.getBusInfo(AUDIO, OUTPUT, 0, &mut bus), kResultOk);
        assert_eq!(bus.channelCount, 2);
    }
    assert_eq!(host.layout(), Layout::new(1, 2));
    let input = [3.0f32; 4];
    let (mut left, mut right) = ([9.0f32; 4], [9.0f32; 4]);
    assert_eq!(
        host.process(&[&input], &mut [&mut left, &mut right], 4, &[]),
        Ok(())
    );
    assert_eq!((left, right), ([4.0; 4], [1.0; 4]));
}

/// A queue for `id`, the gain's unless a test names another, claiming
/// `points` points: point `i` at offset `i`, the last at 0.75 and the rest
/// at 0.25, or, when `failing`, refused. Counts the points read.
struct Claimed {
    id: ParamID,
    points: int32,
    failing: bool,
    read: Cell<u64>,
}

impl Class for Claimed {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Claimed {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        self.read.set(self.read.get() + 1);
        if self.failing && index != self.points - 1 {
            return kResultFalse;
        }
        // SAFETY: the exporter passes room for the point.
        unsafe {
            *offset = index;
            *value = if index == self.points - 1 { 0.75 } else { 0.25 };
        }
        kResultOk
    }

    unsafe fn addPoint(&self, _offset: int32, _value: ParamValue, _index: *mut int32) -> tresult {
        kResultFalse
    }
}

/// A block's changes claiming `queues` queues, each the same queue.
/// Counts the queues asked for.
struct Queues {
    queues: int32,
    queue: ComWrapper<Claimed>,
    asked: Cell<u64>,
}

impl Queues {
    fn new(queues: int32, points: int32) -> ComWrapper<Self> {
        Self::claiming(queues, points, false)
    }

    fn claiming(queues: int32, points: int32, failing: bool) -> ComWrapper<Self> {
        Self::of(queues, GAIN, points, failing)
    }

    /// `queues` queues of one point each for `id`.
    fn claiming_id(queues: int32, id: ParamID) -> ComWrapper<Self> {
        Self::of(queues, id, 1, false)
    }

    fn of(queues: int32, id: ParamID, points: int32, failing: bool) -> ComWrapper<Self> {
        ComWrapper::new(Queues {
            queues,
            queue: ComWrapper::new(Claimed {
                id,
                points,
                failing,
                read: Cell::new(0),
            }),
            asked: Cell::new(0),
        })
    }

    /// Processes a block of no frames with these changes.
    fn send(this: &ComWrapper<Self>, host: &TestHost<Probe>) {
        let (_, processor, _) = host.interfaces();
        // SAFETY: all zeros is valid process data: no buses and no frames.
        let mut data: ProcessData = unsafe { std::mem::zeroed() };
        data.inputParameterChanges = this
            .as_com_ref::<IParameterChanges>()
            .expect("changes")
            .as_ptr();
        // SAFETY: a call on a live instance with data, and changes, that
        // live through it.
        assert_eq!(unsafe { processor.process(&mut data) }, kResultOk);
    }
}

impl Class for Queues {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for Queues {
    unsafe fn getParameterCount(&self) -> int32 {
        self.queues
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        self.asked.set(self.asked.get() + 1);
        match (
            index < self.queues,
            self.queue.as_com_ref::<IParamValueQueue>(),
        ) {
            (true, Some(queue)) => queue.as_ptr(),
            _ => std::ptr::null_mut(),
        }
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        _index: *mut int32,
    ) -> *mut IParamValueQueue {
        std::ptr::null_mut()
    }
}

#[test]
fn the_last_point_of_a_queue_wins() {
    let host = running(64);
    Queues::send(&Queues::new(1, 2), &host);
    assert_eq!(host.normalized(GAIN), 0.75);
}

#[test]
fn every_point_keeps_its_offset_and_each_queue_lands_its_last() {
    let mut host = running(64);
    let mut left = [0.0f32; 8];
    let gain: &[(i32, f64)] = &[(0, 0.25), (4, 0.5), (7, 0.75)];
    assert_eq!(
        host.process_queues(&mut [&mut left], 8, &[(GAIN, gain), (MODE, &[(2, 1.0)])]),
        Ok(())
    );
    assert_eq!((host.normalized(GAIN), host.normalized(MODE)), (0.75, 1.0));
    assert_eq!(left, [12.0; 8], "one value for the whole block");
    let (points, dropped) = host.component().carried();
    let carried: Vec<(u32, u32, f64)> = points
        .iter()
        .map(|point| (host_id_of(point.id), point.offset, point.normalized))
        .collect();
    assert_eq!(
        carried,
        [
            (GAIN, 0, 0.25),
            (GAIN, 4, 0.5),
            (GAIN, 7, 0.75),
            (MODE, 2, 1.0)
        ]
    );
    assert_eq!(dropped, 0);
}

/// The host id of a parameter of the probe.
fn host_id_of(id: ParamId) -> u32 {
    host_id(PARAMS[id.index()].key)
}

#[test]
fn a_host_claiming_more_queues_than_parameters_is_walked_no_further() {
    // Once a queue named each of the probe's three parameters, the walk is
    // over: a fourth is not read, though it names the gain again.
    let mut host = running(64);
    let mut left = [0.0f32; 8];
    let queues: &[(u32, &[(i32, f64)])] = &[
        (GAIN, &[(0, 0.25)]),
        (MODE, &[(2, 1.0)]),
        (BYPASS, &[(6, 1.0)]),
        (GAIN, &[(4, 0.75)]),
    ];
    assert_eq!(host.process_queues(&mut [&mut left], 8, queues), Ok(()));
    assert_eq!(host.normalized(GAIN), 0.25);
    assert_eq!(host.component().carried().0.len(), 3);
}

#[test]
fn a_host_claiming_millions_of_queues_for_one_parameter_is_walked_a_bounded_way() {
    // The first takes the gain's place; each repeat is passed over unread,
    // as an unknown id is, up to the same bound.
    let host = running(64);
    let changes = Queues::new(16_000_000, 1);
    Queues::send(&changes, &host);
    assert_eq!(
        changes.asked.get(),
        1 + crate::engine::MAX_SKIPPED_QUEUES as u64
    );
    assert_eq!(changes.queue.read.get(), 1, "only the first queue's point");
    assert_eq!(host.normalized(GAIN), 0.75);
}

#[test]
fn queues_for_ids_the_plugin_does_not_know_never_starve_its_own() {
    // As many unknown ids as the probe has parameters, and an empty queue,
    // listed before its own: a host that lists hidden proxy ids first.
    let unknown: Vec<u32> = (1..)
        .filter(|id| ![GAIN, MODE, BYPASS].contains(id))
        .take(3)
        .collect();
    let mut host = running(64);
    let mut left = [0.0f32; 8];
    let point: &[(i32, f64)] = &[(0, 0.5)];
    let queues: &[(u32, &[(i32, f64)])] = &[
        (unknown[0], point),
        (unknown[1], point),
        (GAIN, &[]),
        (unknown[2], point),
        (GAIN, &[(3, 0.0)]),
        (MODE, &[(5, 1.0)]),
    ];
    assert_eq!(host.process_queues(&mut [&mut left], 8, queues), Ok(()));
    assert_eq!((host.normalized(GAIN), host.normalized(MODE)), (0.0, 1.0));
}

#[test]
fn a_parameter_in_two_queues_takes_one_parameter_s_place_and_its_first_queue_wins() {
    // The walk takes as many queues as the probe has parameters. A host
    // that lists the gain twice must not use up the bypass's place.
    let mut host = running(64);
    let mut left = [0.0f32; 8];
    let queues: &[(u32, &[(i32, f64)])] = &[
        (GAIN, &[(0, 0.25)]),
        (GAIN, &[(4, 0.75)]),
        (MODE, &[(2, 1.0)]),
        (BYPASS, &[(6, 1.0)]),
    ];
    assert_eq!(host.process_queues(&mut [&mut left], 8, queues), Ok(()));
    assert_eq!(
        (
            host.normalized(GAIN),
            host.normalized(MODE),
            host.normalized(BYPASS)
        ),
        (0.25, 1.0, 1.0)
    );
    let (points, dropped) = host.component().carried();
    assert_eq!(
        (points.len(), dropped),
        (3, 0),
        "every point but the repeat's, which is not read"
    );
}

#[test]
fn the_first_queue_for_a_parameter_wins_wherever_the_repeat_comes() {
    let queues: [(u32, &[(i32, f64)]); 3] = [
        (GAIN, &[(0, 0.25)]),
        (MODE, &[(2, 1.0)]),
        (BYPASS, &[(6, 1.0)]),
    ];
    let repeat: (u32, &[(i32, f64)]) = (GAIN, &[(4, 0.75)]);
    // Not after the last: the walk ends once every parameter has its queue,
    // so a repeat there is never reached.
    for at in 1..queues.len() {
        let mut listed = queues.to_vec();
        listed.insert(at, repeat);
        let mut host = running(64);
        let mut left = [0.0f32; 8];
        assert_eq!(host.process_queues(&mut [&mut left], 8, &listed), Ok(()));
        assert_eq!(
            (
                host.normalized(GAIN),
                host.normalized(MODE),
                host.normalized(BYPASS)
            ),
            (0.25, 1.0, 1.0),
            "the repeat at {at}"
        );
    }
}

#[test]
fn a_block_moving_every_proxy_of_learn_s_grid_still_reaches_the_plugin_s_own_queue() {
    // Learn's hidden proxies in VST3, 16 channels by 130 controllers, in
    // the id range no key's hash takes, all moved in one block and listed
    // before the plugin's own queue.
    let grid = 16 * 130;
    let first = *seam_params::RESERVED_HOST_IDS.start();
    assert!(seam_params::RESERVED_HOST_IDS.contains(&(first + grid - 1)));
    let point: &[(i32, f64)] = &[(0, 0.5)];
    let mut queues: Vec<(u32, &[(i32, f64)])> =
        (0..grid).map(|proxy| (first + proxy, point)).collect();
    queues.push((GAIN, &[(3, 0.0)]));
    let mut host = running(64);
    let mut left = [0.0f32; 8];
    assert_eq!(host.process_queues(&mut [&mut left], 8, &queues), Ok(()));
    assert_eq!(host.normalized(GAIN), 0.0, "the plugin's own point landed");
}

#[test]
fn a_host_claiming_millions_of_queues_it_does_not_know_is_walked_a_bounded_way() {
    let stranger = 0xFFFF_FFFF;
    assert!(![GAIN, MODE, BYPASS].contains(&stranger));
    let host = running(64);
    let changes = Queues::claiming_id(16_000_000, stranger);
    Queues::send(&changes, &host);
    assert_eq!(
        changes.asked.get(),
        crate::engine::MAX_SKIPPED_QUEUES as u64
    );
}

#[test]
fn a_queue_claiming_millions_of_points_it_cannot_give_is_read_as_far_as_there_is_room() {
    let host = running(64);
    let changes = Queues::claiming(1, 16_000_000, true);
    Queues::send(&changes, &host);
    assert_eq!(host.normalized(GAIN), 0.75, "the last point still lands");
    let read = changes.queue.read.get();
    assert!(read <= 3 * 8 + 1, "{read} points read");
}

#[test]
fn a_queue_claiming_millions_of_points_is_read_as_far_as_there_is_room() {
    let host = running(64);
    let changes = Queues::new(1, 16_000_000);
    Queues::send(&changes, &host);
    assert_eq!(host.normalized(GAIN), 0.75, "the last point still lands");
    let read = changes.queue.read.get();
    assert!(read <= 3 * 8 + 1, "{read} points read");
    let (points, dropped) = host.component().carried();
    assert_eq!(points.last().map(|point| point.normalized), Some(0.75));
    assert_eq!(dropped, 16_000_000 - points.len() as u64);
}

#[test]
fn a_64_bit_block_is_refused_unprocessed() {
    let host = running(64);
    let (_, processor, _) = host.interfaces();
    let mut left = [5.0f32; 8];
    let mut channels = [left.as_mut_ptr()];
    let mut bus = AudioBusBuffers {
        numChannels: 1,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: channels.as_mut_ptr(),
        },
    };
    // SAFETY: all zeros is valid process data, filled in below.
    let mut data: ProcessData = unsafe { std::mem::zeroed() };
    data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as int32;
    data.numSamples = 8;
    data.numOutputs = 1;
    data.outputs = &mut bus;
    // SAFETY: the component is alive while the host holds its interfaces;
    // `data`, its one output bus and that bus's channel list live through
    // the call, the channel holding the 8 floats the block claims, and the
    // test touches none of them meanwhile.
    assert_eq!(unsafe { processor.process(&mut data) }, kResultFalse);
    assert_eq!(left, [5.0; 8], "the host's buffer as it was");
    assert_eq!(probe(|state| state.blocks.len()), 0, "no block ran");
}

#[test]
fn a_latency_activation_sets_reaches_a_host_that_asked_before() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    assert_eq!(
        host.latency(),
        0,
        "asked before activating, as JUCE's hosting does"
    );
    probe(|state| state.latency = 64);
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.latency(), 64, "answered at once");
    host.tick();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_LATENCY)]);
    host.tick();
    assert_eq!(host.calls(), [], "told once");
}

#[test]
fn a_controller_write_after_deactivation_lands() {
    let host = running(64);
    assert_eq!(
        host.deactivate(),
        Ok(()),
        "with no setProcessing(false) first"
    );
    assert_eq!(host.set_normalized(GAIN, 0.25), Ok(()));
    assert_eq!(host.normalized(GAIN), 0.25);
}

#[test]
fn setup_is_refused_while_active() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.setup(96_000.0, 32), Err(Refused(kResultFalse)));
    assert_eq!(host.deactivate(), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    let setups = probe(|state| {
        state
            .setups
            .iter()
            .map(|setup| (setup.sample_rate, setup.max_frames))
            .collect::<Vec<_>>()
    });
    assert_eq!(setups, [(48_000.0, 64), (48_000.0, 64)]);
}

#[test]
fn layout_latency_and_tail_answer_on_any_thread() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.arrange(Layout::MONO), Ok(()));
    probe(|state| state.latency = 64);
    host.tick();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_LATENCY)]);
    let (plugin, processor, _) = host.interfaces();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let faults = thread_faults();
            // SAFETY: the component is alive while `plugin` and `processor`
            // hold their references; `arrangement` and `bus` are live and
            // valid for the calls that write them; these calls may arrive on
            // any thread.
            unsafe {
                assert_eq!(processor.getLatencySamples(), 64);
                assert_eq!(processor.getTailSamples(), 100);
                let mut arrangement: SpeakerArrangement = 0;
                assert_eq!(
                    processor.getBusArrangement(OUTPUT, 0, &mut arrangement),
                    kResultOk
                );
                assert_eq!(arrangement, SpeakerArr::kMono);
                let mut bus: BusInfo = std::mem::zeroed();
                assert_eq!(plugin.getBusInfo(AUDIO, INPUT, 0, &mut bus), kResultOk);
                assert_eq!(bus.channelCount, 1);
            }
            assert_eq!(thread_faults(), faults, "none refused");
        });
    });
}

#[test]
fn a_new_handler_ends_the_old_ones_open_brackets() {
    let host = TestHost::<Probe>::new();
    probe(|state| state.hold = true);
    host.tick();
    assert_eq!(
        host.calls(),
        [
            HostCall::Begin(GAIN),
            HostCall::Perform(GAIN, PARAMS[0].kind.normalize(3.0))
        ]
    );
    let other = ComWrapper::new(Recorder::default());
    let (_, _, controller) = host.interfaces();
    let pointer = other
        .as_com_ref::<IComponentHandler>()
        .expect("a handler")
        .as_ptr();
    // SAFETY: the component is alive while the host holds its interfaces,
    // `pointer` is a live `IComponentHandler` the component takes a
    // reference to, and this is the component's main thread.
    let switched = unsafe { controller.setComponentHandler(pointer) };
    assert_eq!(switched, kResultOk);
    assert_eq!(host.calls(), [HostCall::End(GAIN)], "told to the old one");
    assert_eq!(other.calls.borrow().len(), 0);
}

#[test]
fn a_sync_made_while_no_handler_is_linked_is_told_to_the_next_one() {
    let host = TestHost::<Probe>::new();
    let (_, _, controller) = host.interfaces();
    // SAFETY: the component is alive while the host holds its interfaces,
    // a null handler unlinks the host's, and this is the main thread.
    let unlinked = unsafe { controller.setComponentHandler(std::ptr::null_mut()) };
    assert_eq!(unlinked, kResultOk);
    probe(|state| state.sync = Some(("mode", 0.0)));
    host.tick();
    let next = ComWrapper::new(Recorder::default());
    let pointer = next
        .as_com_ref::<IComponentHandler>()
        .expect("a handler")
        .as_ptr();
    // SAFETY: as above, and `pointer` is a live `IComponentHandler` the
    // component takes a reference to.
    let linked = unsafe { controller.setComponentHandler(pointer) };
    assert_eq!(linked, kResultOk);
    host.tick();
    assert_eq!(*next.calls.borrow(), [HostCall::Restart(RESTART_VALUES)]);
}

/// A component, initialized on this thread, with no timer.
fn initialized() -> ComWrapper<Component<Probe>> {
    let component = Component::<Probe>::wrap(false);
    let plugin = component.as_com_ref::<IComponent>().expect("a component");
    // SAFETY: a call on a live component, passing no pointer.
    let initialized = unsafe { plugin.initialize(std::ptr::null_mut()) };
    assert_eq!(initialized, kResultOk);
    component
}

#[test]
fn the_plugin_is_built_at_initialize_on_the_thread_that_calls_it() {
    let component = std::thread::spawn(|| Component::<Probe>::wrap(false))
        .join()
        .expect("created on another thread");
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    let controller = component
        .to_com_ptr::<IEditController>()
        .expect("its own controller");
    // SAFETY: the component is alive while `plugin` and `controller` hold
    // their references, and the calls pass no pointer.
    unsafe {
        assert_eq!(controller.getParameterCount(), 0, "nothing before it");
        assert_eq!(plugin.setActive(1), kNotInitialized);
        assert_eq!(plugin.initialize(std::ptr::null_mut()), kResultOk);
        assert_eq!(controller.getParameterCount(), 3);
    }
    let (plugin, faults) = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let faults = thread_faults();
                // SAFETY: as above, from a thread that is not the main one.
                let answers =
                    unsafe { (plugin.initialize(std::ptr::null_mut()), plugin.setActive(1)) };
                (answers, thread_faults() - faults)
            })
            .join()
            .expect("the other thread")
    });
    assert_eq!(plugin, (kResultFalse, kResultFalse), "refused elsewhere");
    assert_eq!(faults, 2);
    assert_eq!(probe(|state| state.setups.len()), 0);
}

/// Activates a component, releases it on another thread, and answers the
/// faults counted there, the lines written there, and the tick it left
/// behind.
fn release_elsewhere(panic_in_processor_drop: bool) -> (u64, Vec<String>, Box<dyn Fn()>) {
    let component = initialized();
    let tick = component.ticker();
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    drop(component);
    probe(|state| state.panic_in_processor_drop = panic_in_processor_drop);
    // SAFETY: a call on a live component, on its main thread, passing no
    // pointer.
    unsafe {
        assert_eq!(plugin.setActive(1), kResultOk);
    }
    let (faults, lines) = std::thread::spawn(move || {
        let faults = thread_faults();
        written();
        drop(plugin);
        (thread_faults() - faults, written())
    })
    .join()
    .expect("released there, the host not aborted");
    (faults, lines, tick)
}

#[test]
fn a_release_on_another_thread_leaks_the_plugin_drops_the_processor_and_counts() {
    let (faults, lines, tick) = release_elsewhere(false);
    assert_eq!(faults, 1);
    assert!(
        lines.len() == 1 && lines[0].contains("released there and leaked"),
        "reported where it was released: {lines:?}"
    );
    assert_eq!(
        probe(|state| (state.deactivations, state.drops)),
        (0, 0),
        "the plugin was never called or dropped off its thread"
    );
    assert_eq!(
        probe(|state| state.processor_drops.load(Ordering::Relaxed)),
        1,
        "the processor dropped there"
    );
    tick();
    assert_eq!(probe(|state| state.idles), 0, "nor ticked after");
}

#[test]
fn a_processor_that_panics_as_it_drops_on_another_thread_is_caught_there() {
    let (faults, _, _) = release_elsewhere(true);
    assert_eq!(faults, 2, "the release, and the panic it caught");
    assert_eq!(
        probe(|state| state.processor_drops.load(Ordering::Relaxed)),
        1
    );
}

#[test]
fn a_block_with_no_output_buffers_is_answered() {
    for active in [false, true] {
        let host = TestHost::<Probe>::new();
        assert_eq!(host.setup(48_000.0, 64), Ok(()));
        if active {
            assert_eq!(host.activate(), Ok(()));
        }
        let (_, processor, _) = host.interfaces();
        // SAFETY: all zeros is valid process data: buses counted, none
        // given.
        let mut data: ProcessData = unsafe { std::mem::zeroed() };
        data.symbolicSampleSize = SymbolicSampleSizes_::kSample32 as int32;
        data.numSamples = 8;
        data.numOutputs = 2;
        // SAFETY: the component is alive while the host holds its
        // interfaces, and `data` lives through the call, its null bus lists
        // claiming buses it does not give, which the call must answer
        // without reading.
        let processed = unsafe { processor.process(&mut data) };
        assert_eq!(processed, kResultOk, "active: {active}");
    }
}

#[test]
fn a_tick_that_panicked_leaves_the_instance_to_drop_normally() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.activate(), Ok(()));
    probe(|state| state.panic_in_idle = true);
    host.expect_faults(1);
    host.tick();
    probe(|state| state.panic_in_idle = false);
    drop(host);
    assert_eq!(probe(|state| state.deactivations), 1);
    assert_eq!(probe(|state| state.drops), 1, "freed, not leaked");
}

/// A host's handler that holds the instance's last reference and drops it
/// at the first edit it hears of, inside the tick that sends it.
#[derive(Default)]
struct Releaser {
    instance: RefCell<Option<ComPtr<IComponent>>>,
    calls: RefCell<Vec<HostCall>>,
}

impl Class for Releaser {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Releaser {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.calls.borrow_mut().push(HostCall::Begin(id));
        drop(self.instance.borrow_mut().take());
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, normalized: ParamValue) -> tresult {
        self.calls
            .borrow_mut()
            .push(HostCall::Perform(id, normalized));
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.calls.borrow_mut().push(HostCall::End(id));
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.calls.borrow_mut().push(HostCall::Restart(flags));
        kResultOk
    }
}

#[test]
fn a_release_inside_a_tick_leaks_what_the_tick_still_uses() {
    let faults = thread_faults();
    let component = initialized();
    let tick = component.ticker();
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    let controller = component
        .to_com_ptr::<IEditController>()
        .expect("its own controller");
    drop(component);
    let releaser = ComWrapper::new(Releaser::default());
    let handler = releaser
        .as_com_ref::<IComponentHandler>()
        .expect("a handler")
        .as_ptr();
    // SAFETY: the component is alive while `plugin` and `controller` hold
    // their references, `handler` is a live `IComponentHandler` the component
    // takes a reference to, and this is the component's main thread.
    unsafe {
        assert_eq!(controller.setComponentHandler(handler), kResultOk);
        assert_eq!(plugin.setActive(1), kResultOk);
    }
    drop(controller);
    *releaser.instance.borrow_mut() = Some(plugin);

    probe(|state| state.edit = Some(("gain", 6.0)));
    tick();
    assert!(releaser.instance.borrow().is_none(), "released inside");
    assert_eq!(
        *releaser.calls.borrow(),
        [HostCall::Begin(GAIN)],
        "the tick finished without calling the host again"
    );
    assert_eq!(probe(|state| state.deactivations), 1, "deactivated");
    assert_eq!(probe(|state| state.drops), 0, "and leaked");
    tick();
    assert_eq!(probe(|state| state.idles), 1, "and ticks no more");
    assert_eq!(thread_faults(), faults);
}

/// A host's handler that ticks the instance again from `beginEdit`, as a
/// nested message loop would, and drops the instance's last reference at
/// `performEdit`, inside the outer tick.
#[derive(Default)]
struct NestedReleaser {
    instance: RefCell<Option<ComPtr<IComponent>>>,
    tick: RefCell<Option<Box<dyn Fn()>>>,
    calls: RefCell<Vec<HostCall>>,
}

impl Class for NestedReleaser {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for NestedReleaser {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.calls.borrow_mut().push(HostCall::Begin(id));
        if let Some(tick) = self.tick.borrow().as_ref() {
            tick();
        }
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, normalized: ParamValue) -> tresult {
        self.calls
            .borrow_mut()
            .push(HostCall::Perform(id, normalized));
        drop(self.instance.borrow_mut().take());
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.calls.borrow_mut().push(HostCall::End(id));
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.calls.borrow_mut().push(HostCall::Restart(flags));
        kResultOk
    }
}

#[test]
fn a_tick_nested_inside_a_tick_does_nothing() {
    let faults = thread_faults();
    let component = initialized();
    let tick = component.ticker();
    let nested = component.ticker();
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    let controller = component
        .to_com_ptr::<IEditController>()
        .expect("its own controller");
    drop(component);
    let releaser = ComWrapper::new(NestedReleaser::default());
    let handler = releaser
        .as_com_ref::<IComponentHandler>()
        .expect("a handler")
        .as_ptr();
    // SAFETY: the component is alive while `plugin` and `controller` hold
    // their references, `handler` is a live `IComponentHandler` the component
    // takes a reference to, and this is the component's main thread.
    unsafe {
        assert_eq!(controller.setComponentHandler(handler), kResultOk);
        assert_eq!(plugin.setActive(1), kResultOk);
    }
    drop(controller);
    *releaser.instance.borrow_mut() = Some(plugin);
    *releaser.tick.borrow_mut() = Some(nested);

    probe(|state| state.edit = Some(("gain", 6.0)));
    tick();
    assert_eq!(probe(|state| state.idles), 1, "the nested tick did nothing");
    assert!(releaser.instance.borrow().is_none(), "released inside");
    assert_eq!(
        probe(|state| state.drops),
        0,
        "left to leak, since the outer tick was still running"
    );
    assert_eq!(probe(|state| state.deactivations), 1, "deactivated");
    assert_eq!(thread_faults(), faults);
}

/// A participant that keeps the section a restore hands it.
struct Kept(Rc<RefCell<Option<serde_json::Value>>>);

impl seam_session::Participant for Kept {
    fn capture(&self) -> seam_session::Section {
        self.0.borrow().clone().unwrap_or(serde_json::Value::Null)
    }

    fn apply(
        &mut self,
        section: Option<&seam_session::Section>,
    ) -> Result<(), seam_session::ApplyError> {
        *self.0.borrow_mut() = section.cloned();
        Ok(())
    }
}

thread_local! {
    /// What the session plugin's participant was handed, and its ticks,
    /// on the thread that ticks it.
    static KEPT: Rc<RefCell<Option<serde_json::Value>>> = Rc::default();
    static TICKS: Cell<u32> = const { Cell::new(0) };
}

/// A plugin with one session participant, which Seam's tick applies.
struct WithSession {
    _parts: Parts,
    _participant: Rc<RefCell<Kept>>,
    _registration: seam_session::Registration,
}

impl Plugin for WithSession {
    const INFO: PluginInfo = Probe::INFO;
    type Params = Table;
    type Processor = ShapedProcessor;

    fn new(_: Table, parts: Parts) -> Self {
        let participant = Rc::new(RefCell::new(Kept(KEPT.with(Rc::clone))));
        let registration = parts
            .session()
            .register("kept", &participant)
            .expect("registered");
        WithSession {
            _parts: parts,
            _participant: participant,
            _registration: registration,
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> ShapedProcessor {
        ShapedProcessor
    }

    fn idle(&mut self) {
        TICKS.with(|ticks| ticks.set(ticks.get() + 1));
    }
}

/// The exporter's own timer ticks the plugin through this thread's loop,
/// with no tick by hand: on macOS, the run loop timer, which CI checks only
/// there.
#[test]
#[cfg_attr(
    any(miri, not(any(windows, target_os = "macos"))),
    ignore = "no system timer under miri or on this platform"
)]
fn the_timer_ticks_the_plugin_and_a_host_load_reaches_its_session() {
    let host = TestHost::<WithSession>::with_timer();
    assert!(host.pump_until(|| TICKS.with(Cell::get) >= 3), "ticked");
    let block = br#"{"seamStateVersion": 1, "parameters": {}, "extra": {"kept": 42}}"#;
    assert_eq!(host.load(block), Ok(()));
    assert!(
        host.pump_until(|| KEPT.with(|kept| *kept.borrow() == Some(serde_json::json!(42)))),
        "applied by a tick"
    );
}

/// A host as impolite as real ones get, all at once: the instance created
/// on one thread and initialized on this one, its own timer ticking it
/// through this thread's loop, and blocks with queues of points at offsets
/// on a third thread while this one pumps.
#[test]
#[cfg_attr(
    any(miri, not(any(windows, target_os = "macos"))),
    ignore = "no system timer under miri or on this platform"
)]
fn a_host_on_three_threads_with_real_ticks_and_points_at_offsets() {
    let host = TestHost::<Probe>::with(super::Options {
        created_elsewhere: true,
        timer: true,
    });
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    probe(|state| state.edit = Some(("gain", 6.0)));
    let side = host.audio_side();
    let edit = PARAMS[0].kind.normalize(6.0);
    std::thread::scope(|scope| {
        let audio = scope.spawn(move || {
            let mut side = side;
            let end = Instant::now() + Duration::from_secs(10);
            // Until a block hears the edit the main thread's tick makes.
            for block in 0u32.. {
                let mode: &[(i32, f64)] = &[(1, 0.0), (5, f64::from(block % 2))];
                let mut left = [1.0f32; 8];
                assert_eq!(
                    side.process_queues(&mut [&mut left], 8, &[(MODE, mode)]),
                    Ok(())
                );
                if left == [7.0; 8] || Instant::now() > end {
                    return (left, f64::from(block % 2));
                }
                std::thread::sleep(Duration::from_micros(250));
            }
            unreachable!("the loop returns")
        });
        let told = host.pump_until(|| host.calls().contains(&HostCall::Perform(GAIN, edit)));
        assert!(told, "the timer's tick told the host of the plugin's edit");
        let (last, mode) = audio.join().expect("the audio thread");
        assert_eq!(last, [7.0; 8], "the edit reached the audio thread");
        assert_eq!(host.normalized(MODE), mode, "the last block's last point");
    });
    let (points, dropped) = host.component().carried();
    assert_eq!(
        points.iter().map(|point| point.offset).collect::<Vec<_>>(),
        [1, 5]
    );
    assert_eq!(dropped, 0);
}

#[test]
fn a_setup_with_no_block_size_keeps_its_rate_and_one_past_the_limit_is_built_for_the_limit() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.setup(96_000.0, 0), Ok(()), "a rate with no block size");
    assert_eq!(
        host.setup(f64::NAN, 64),
        Err(Refused(kInvalidArgument)),
        "no rate"
    );
    assert_eq!(host.activate(), Ok(()));
    let setups = probe(|state| state.setups.clone());
    assert_eq!(
        (setups[0].sample_rate, setups[0].max_frames),
        (96_000.0, 1024),
        "the rate kept, the block size the fallback's"
    );
    // One past the limit, and the most a VST3 setup can promise.
    for promised in [crate::MAX_FRAMES + 1, usize::MAX] {
        assert_eq!(host.deactivate(), Ok(()));
        assert_eq!(host.setup(44_100.0, promised), Ok(()), "{promised} frames");
        assert_eq!(host.activate(), Ok(()));
        let setup = probe(|state| state.setups.last().copied()).expect("built");
        assert_eq!(
            (setup.sample_rate, setup.max_frames),
            (44_100.0, crate::MAX_FRAMES),
            "{promised} frames: longer blocks play in pieces"
        );
    }
}

/// A host's stream that never ends.
struct Endless;

impl Class for Endless {
    type Interfaces = (IBStream,);
}

impl ::vst3::Steinberg::IBStreamTrait for Endless {
    unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
        // SAFETY: the reader passes room for `count` bytes, and for the
        // count read.
        unsafe {
            std::ptr::write_bytes(
                buffer.cast::<u8>(),
                b' ',
                usize::try_from(count).unwrap_or(0),
            );
            *read = count;
        }
        kResultOk
    }

    unsafe fn write(&self, _: *mut c_void, _: int32, _: *mut int32) -> tresult {
        kNotImplemented
    }

    unsafe fn seek(
        &self,
        _: ::vst3::Steinberg::int64,
        _: int32,
        _: *mut ::vst3::Steinberg::int64,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn tell(&self, _: *mut ::vst3::Steinberg::int64) -> tresult {
        kNotImplemented
    }
}

#[test]
#[cfg_attr(miri, ignore = "64 MiB of stream is too slow to interpret")]
fn an_endless_state_stream_fails_the_load() {
    let host = TestHost::<Probe>::new();
    let (plugin, _, _) = host.interfaces();
    let stream = ComWrapper::new(Endless);
    let pointer = stream.as_com_ref::<IBStream>().expect("a stream").as_ptr();
    // SAFETY: a call on a live instance with a stream that outlives it.
    assert_eq!(unsafe { plugin.setState(pointer) }, kResultFalse);
}

#[test]
fn every_arrangement_offered_is_one_the_exporter_then_takes() {
    let host = TestHost::<Probe>::new();
    let (_, processor, _) = host.interfaces();
    for layout in [Layout::STEREO, Layout::MONO] {
        assert_eq!(host.arrange(layout), Ok(()));
        let (mut input, mut output): (SpeakerArrangement, SpeakerArrangement) = (0, 0);
        // SAFETY: the component is alive while the host holds its
        // interfaces, and `input` and `output` are each one live arrangement,
        // which `getBusArrangement` writes and `setBusArrangements` reads.
        unsafe {
            assert_eq!(processor.getBusArrangement(INPUT, 0, &mut input), kResultOk);
            assert_eq!(
                processor.getBusArrangement(OUTPUT, 0, &mut output),
                kResultOk
            );
            assert_eq!(
                processor.setBusArrangements(&mut input, 1, &mut output, 1),
                ::vst3::Steinberg::kResultTrue,
                "{layout:?}"
            );
        }
    }
}

#[test]
fn a_bus_wider_than_stereo_is_refused_by_name() {
    // Each of these is wide on one side only, or on both.
    for (inputs, outputs) in [(6, 6), (2, 6), (6, 2)] {
        let refused = std::panic::catch_unwind(|| {
            super::check_layouts(&[
                Layout::STEREO,
                Layout::new(1, 2),
                Layout::new(inputs, outputs),
            ]);
        });
        let message = refused.expect_err("refused");
        let message = message
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or_default();
        let expected = format!("layout 2 has {inputs} in and {outputs} out");
        assert!(message.contains(&expected), "{message}");
    }
    super::check_layouts(&[Layout::new(0, 2), Layout::MONO]);
}

/// A component with its own timer, initialized on this thread, which runs
/// the timer; and the timer's handle.
fn ticking() -> (ComWrapper<Component<Probe>>, crate::engine::timer::Handle) {
    let component = Component::<Probe>::wrap(true);
    let plugin = component.as_com_ref::<IComponent>().expect("a component");
    // SAFETY: a call on a live component, passing no pointer.
    let initialized = unsafe { plugin.initialize(std::ptr::null_mut()) };
    assert_eq!(initialized, kResultOk);
    let handle = component.timer_handle().expect("a timer");
    (component, handle)
}

/// Runs this thread's loop until `done` holds, for ten seconds at most.
fn pump_until(mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while !done() && Instant::now() < end {
        pump(Duration::from_millis(10));
    }
    done()
}

/// The tick's pointer to the engine is valid only while the timer that
/// holds it lives: a release that frees the engine must stop it first.
#[test]
#[cfg_attr(
    any(miri, not(any(windows, target_os = "macos"))),
    ignore = "no system timer under miri or on this platform"
)]
fn no_tick_reaches_a_released_instance() {
    let host = TestHost::<Probe>::with_timer();
    assert!(
        host.pump_until(|| probe(|state| state.idles) >= 2),
        "ticked"
    );
    // A released timer is freed. Windows can still ask its window, which
    // reads as destroyed; a Core Foundation timer cannot be asked after its
    // last reference is gone, so there the silence below is the proof.
    #[cfg(windows)]
    let handle = host.component().timer_handle().expect("a timer");
    drop(host);
    #[cfg(windows)]
    assert!(detached(handle), "the timer went with the instance");
    let idles = probe(|state| state.idles);
    pump(Duration::from_millis(200));
    assert_eq!(
        probe(|state| state.idles),
        idles,
        "no tick after the release"
    );
}

/// A release from a host callback inside a tick leaks the engine, timer
/// included, and must detach the timer from its tick.
#[test]
#[cfg_attr(
    any(miri, not(any(windows, target_os = "macos"))),
    ignore = "no system timer under miri or on this platform"
)]
fn a_release_inside_a_real_tick_detaches_the_timer() {
    let (component, handle) = ticking();
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    let controller = component
        .to_com_ptr::<IEditController>()
        .expect("its own controller");
    drop(component);
    let releaser = ComWrapper::new(Releaser::default());
    let handler = releaser
        .as_com_ref::<IComponentHandler>()
        .expect("a handler")
        .as_ptr();
    // SAFETY: the component is alive while `plugin` and `controller` hold
    // their references, `handler` is a live `IComponentHandler` the
    // component takes a reference to, and this is the component's main
    // thread.
    unsafe {
        assert_eq!(controller.setComponentHandler(handler), kResultOk);
        assert_eq!(plugin.setActive(1), kResultOk);
    }
    drop(controller);
    *releaser.instance.borrow_mut() = Some(plugin);
    probe(|state| state.edit = Some(("gain", 6.0)));
    assert!(
        pump_until(|| releaser.instance.borrow().is_none()),
        "released inside a tick"
    );
    assert!(
        detached(handle),
        "the leaked timer reaches its tick no more"
    );
    assert_eq!(probe(|state| state.drops), 0, "the plugin leaked");
}

/// A release on another thread leaks the engine, timer included, and must
/// detach the timer from its tick from there.
#[test]
#[cfg_attr(
    any(miri, not(any(windows, target_os = "macos"))),
    ignore = "no system timer under miri or on this platform"
)]
fn a_release_on_another_thread_detaches_the_timer() {
    let (component, handle) = ticking();
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    drop(component);
    let faults = std::thread::spawn(move || {
        let faults = thread_faults();
        drop(plugin);
        thread_faults() - faults
    })
    .join()
    .expect("released there");
    assert_eq!(faults, 1);
    assert!(
        detached(handle),
        "the leaked timer reaches its tick no more"
    );
    pump(Duration::from_millis(50));
    assert_eq!(probe(|state| state.drops), 0, "the plugin leaked");
}

/// A plugin whose migration refuses every older block, with a reason.
struct Stuck;

struct StuckProcessor;

impl Processor for StuckProcessor {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Continue
    }
}

impl Plugin for Stuck {
    const INFO: PluginInfo = Probe::INFO;
    type Params = Table;
    const SCHEMA_VERSION: u64 = 1;
    type Processor = StuckProcessor;

    fn new(_: Table, _: Parts) -> Self {
        Stuck
    }

    fn migrate(_: u64, _: &mut Map<String, Json>) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("no whole steps".into())
    }

    fn activate(&mut self, _: &ProcessSetup) -> StuckProcessor {
        StuckProcessor
    }
}

#[test]
fn the_first_refused_load_writes_the_reason_and_is_no_fault_through_the_vst3_exporter() {
    let host = TestHost::<Stuck>::new();
    let older = br#"{"seamStateVersion":1,"pluginStateVersion":0,"parameters":{}}"#;
    let faults = thread_faults();
    written();
    assert_eq!(
        host.load(older),
        Err(Refused(kResultFalse)),
        "the host's answer stands"
    );
    let lines = written();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("seam: Probe (instance "), "{lines:?}");
    assert!(
        lines[0].contains("): the host's saved state was refused: "),
        "{lines:?}"
    );
    assert!(
        lines[0].ends_with(": the plugin could not upgrade the saved block: no whole steps."),
        "{lines:?}"
    );
    assert_eq!(
        host.load(older),
        Err(Refused(kResultFalse)),
        "refused again"
    );
    assert_eq!(written(), Vec::<String>::new(), "and said once");
    assert_eq!(thread_faults(), faults, "a refused load is no fault");
}

/// Declares one key twice, which the set refuses.
struct Twice;

impl Params for Twice {
    fn declare(p: &mut Declarer) -> Self {
        p.extend([PARAMS[0], PARAMS[0]]);
        Twice
    }
}

/// A plugin whose declarations are refused, so no instance is built.
struct Unbuilt;

impl Plugin for Unbuilt {
    const INFO: PluginInfo = Probe::INFO;
    type Params = Twice;
    type Processor = ShapedProcessor;

    fn new(_: Twice, _: Parts) -> Self {
        unreachable!("never built")
    }

    fn activate(&mut self, _: &ProcessSetup) -> ShapedProcessor {
        unreachable!("never built")
    }
}

#[test]
fn a_refused_declaration_builds_no_instance_and_says_why_through_the_vst3_exporter() {
    let faults = thread_faults();
    written();
    let component = Component::<Unbuilt>::wrap(false);
    let plugin = component.to_com_ptr::<IComponent>().expect("a component");
    // SAFETY: the component was just made and `plugin` holds it;
    // `initialize` takes a null context.
    let initialized = unsafe { plugin.initialize(std::ptr::null_mut()) };
    assert_eq!(initialized, kInternalError, "no instance");
    assert_eq!(thread_faults(), faults + 1, "the refusal counts as a fault");
    let lines = written();
    let refusal = "seam: Probe: no instance was built, since a declaration was refused: \
                   key \"gain\" is declared twice.";
    assert!(lines.iter().any(|line| line == refusal), "{lines:?}");
}
