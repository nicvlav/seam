//! A VST3 host inside the test's own process: the host's side (streams,
//! parameter queues, the component handler) in Rust, driving the plugin
//! through the exporter's own vtables.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use ::vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirection, BusDirections_, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentHandler, IComponentHandlerTrait, IComponentTrait,
    IEditController, IEditControllerTrait, IParamValueQueue, IParamValueQueueTrait,
    IParameterChanges, IParameterChangesTrait, MediaTypes_, ParamID, ParamValue, ParameterInfo,
    ParameterInfo_::ParameterFlags_, ProcessData, ProcessModes_, ProcessSetup as VstSetup,
    SpeakerArr, SpeakerArrangement, String128, SymbolicSampleSizes_,
};
use ::vst3::Steinberg::{
    IBStream, IBStreamTrait, IPluginBaseTrait, int32, int64, kInternalError, kInvalidArgument,
    kNoInterface, kNotImplemented, kNotInitialized, kOutOfMemory, kResultFalse, kResultOk, tresult,
};
use ::vst3::{Class, ComPtr, ComWrapper};

use super::component::Component;
use super::text::copy_utf16;
use crate::engine::timer::pump;
use crate::{Layout, Plugin};

/// VST3's restart flag asking the host to re-read every parameter value.
pub const RESTART_VALUES: i32 = 4;
/// VST3's restart flag saying the latency changed.
pub const RESTART_LATENCY: i32 = 8;

/// What the plugin told the host through its component handler.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::{HostCall, TestHost};
///
/// let host = TestHost::<Gain>::new();
/// let id = host.parameter(0).expect("listed").id;
///
/// // The plugin's UI drags the gain to +6 dB.
/// EDIT.store(true, Ordering::Relaxed);
/// host.tick();
/// // The host hears the whole gesture, the value as 0..1.
/// assert_eq!(
///     host.calls(),
///     [HostCall::Begin(id), HostCall::Perform(id, 0.625), HostCall::End(id)]
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostCall {
    /// `beginEdit`, with the parameter's host id.
    Begin(u32),
    /// `performEdit`: the host id and the 0..1 value.
    Perform(u32, f64),
    /// `endEdit`.
    End(u32),
    /// `restartComponent`, with its flags ([`RESTART_VALUES`],
    /// [`RESTART_LATENCY`]).
    Restart(i32),
}

/// A call the plugin refused, and the `tresult` it answered: anything but
/// `kResultOk`.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::{Refused, TestHost};
///
/// let host = TestHost::<Gain>::new();
/// // A block that is not a saved one: the host is told the load failed.
/// let refused: Refused = host.load(b"not a block").expect_err("refused");
/// assert_eq!(refused.to_string(), "the plugin answered kResultFalse");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub i32);

impl Refused {
    /// `Ok` for `kResultOk`, and the refusal for any other answer.
    fn check(result: tresult) -> Result<(), Refused> {
        if result == kResultOk {
            Ok(())
        } else {
            Err(Refused(result))
        }
    }
}

/// The SDK's names for the answers a plugin refuses a call with.
const REFUSALS: [(tresult, &str); 7] = [
    (kResultFalse, "kResultFalse"),
    (kInvalidArgument, "kInvalidArgument"),
    (kNotImplemented, "kNotImplemented"),
    (kInternalError, "kInternalError"),
    (kNotInitialized, "kNotInitialized"),
    (kOutOfMemory, "kOutOfMemory"),
    (kNoInterface, "kNoInterface"),
];

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match REFUSALS.iter().find(|(result, _)| *result == self.0) {
            Some((_, name)) => write!(f, "the plugin answered {name}"),
            None => write!(f, "the plugin answered the tresult {}", self.0),
        }
    }
}

impl std::error::Error for Refused {}

/// A parameter as the host lists it.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::TestHost;
///
/// let host = TestHost::<Gain>::new();
/// let info = host.parameter(0).expect("listed");
/// assert_eq!((info.title.as_str(), info.units.as_str()), ("Gain", "dB"));
/// assert_eq!(info.step_count, 0, "continuous");
/// assert_eq!(info.default_normalized, 0.5, "0 dB, halfway through -24..24");
/// assert!(info.can_automate && !info.is_list && !info.is_bypass);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    /// The host id.
    pub id: u32,
    /// The name.
    pub title: String,
    /// The unit.
    pub units: String,
    /// VST3's step count: 0 for continuous, else the values minus one.
    pub step_count: i32,
    /// The default, as 0..1.
    pub default_normalized: f64,
    /// Whether the host may automate it.
    pub can_automate: bool,
    /// Whether the host shows it as a list.
    pub is_list: bool,
    /// Whether it is the bypass.
    pub is_bypass: bool,
}

/// How a [`TestHost`] makes its instance.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::{Options, TestHost};
///
/// // A host that creates the instance on another thread, as some do.
/// let host = TestHost::<Gain>::with(Options {
///     created_elsewhere: true,
///     ..Options::default()
/// });
/// assert_eq!(host.parameter_count(), 1);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Create it on another thread, as a host may; it is initialized on the
    /// test's thread all the same.
    pub created_elsewhere: bool,
    /// Start its own timer, as in a host, which only
    /// [`pump_until`](TestHost::pump_until) runs.
    pub timer: bool,
}

/// A VST3 host for tests, holding one instance of `P`.
///
/// It initializes the instance on the test's thread, which becomes the
/// instance's main thread, and makes every call through the component's COM
/// interfaces. [`tick`](Self::tick) stands in for the exporter's timer, or a
/// host made [`with_timer`](Self::with_timer) runs the real one.
///
/// Processing takes the host's buffers either separate or in place, after a
/// list of `(host id, 0..1)` changes, each sent as one queue of one point at
/// the block's start, or [queues of points at sample
/// offsets](Self::process_queues). A block of up to 16 channels and 64
/// queues of up to 64 points allocates nothing in the host.
///
/// Its instance must have no fault, on any thread: it checks when dropped,
/// allowing only the faults a test provokes on purpose and declares with
/// [`expect_faults`](Self::expect_faults).
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::TestHost;
///
/// let mut host = TestHost::<Gain>::new();
/// let id = host.parameter(0).expect("listed").id;
/// host.setup(48_000.0, 4)?;
/// host.activate()?;
/// host.start()?;
///
/// let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
/// // The host sends 0.0 for the gain with the block: -24 dB.
/// let block = &mut [&mut left[..], &mut right[..]];
/// host.process_in_place(block, 4, &[(id, 0.0)])?;
/// assert!((left[0] - 0.0631).abs() < 1e-4);
/// # Ok::<(), seam_host::vst3::Refused>(())
/// ```
pub struct TestHost<P: Plugin> {
    component: ComWrapper<Component<P>>,
    plugin: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: ComPtr<IEditController>,
    handler: ComWrapper<Recorder>,
    blocks: Blocks,
    /// The faults the test provokes on purpose.
    provoked: Cell<u64>,
}

impl<P: Plugin> std::fmt::Debug for TestHost<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHost")
            .field("plugin", &P::INFO.name)
            .finish_non_exhaustive()
    }
}

impl<P: Plugin> Default for TestHost<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Plugin> TestHost<P> {
    /// Creates and initializes an instance, and gives it a component
    /// handler that records what it is told.
    ///
    /// # Panics
    ///
    /// When the plugin's parameter set has its host event drain taken: the
    /// exporter owns it.
    pub fn new() -> Self {
        Self::initialize(ComWrapper::new(Component::<P>::new(false)))
    }

    /// Creates and initializes an instance whose own timer ticks it, as in a
    /// host.
    ///
    /// The timer ticks on this thread's message loop (Windows) or run loop
    /// (macOS), which only [`pump_until`](Self::pump_until) runs. No timer
    /// exists on any other platform.
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    pub fn with_timer() -> Self {
        Self::with(Options {
            timer: true,
            ..Options::default()
        })
    }

    /// Creates and initializes an instance as `options` say.
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    pub fn with(options: Options) -> Self {
        let timer = options.timer;
        let make = move || ComWrapper::new(Component::<P>::new(timer));
        let component = if options.created_elsewhere {
            std::thread::spawn(make)
                .join()
                .expect("created on another thread")
        } else {
            make()
        };
        Self::initialize(component)
    }

    /// Runs this thread's message loop or run loop, as a host's would,
    /// until `done` holds or ten seconds pass, and returns whether it holds:
    /// a host made [`with_timer`](Self::with_timer) ticks meanwhile.
    pub fn pump_until(&self, mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if done() {
                return true;
            }
            pump(Duration::from_millis(10));
        }
        done()
    }

    /// Creates an instance on another thread, as a host may, then
    /// initializes it on this one, which becomes its main thread.
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    pub fn created_elsewhere() -> Self {
        Self::with(Options {
            created_elsewhere: true,
            ..Options::default()
        })
    }

    fn initialize(component: ComWrapper<Component<P>>) -> Self {
        let plugin = component.to_com_ptr::<IComponent>().expect("a component");
        let processor = component
            .to_com_ptr::<IAudioProcessor>()
            .expect("an audio processor");
        let controller = component
            .to_com_ptr::<IEditController>()
            .expect("its own controller");
        let handler = ComWrapper::new(Recorder::default());
        // SAFETY: the component was just made and these interfaces hold it.
        // `initialize` takes a null context; the handler is a live
        // `IComponentHandler`, and the component keeps its own reference.
        unsafe {
            assert_eq!(plugin.initialize(std::ptr::null_mut()), kResultOk);
            let recorder = handler
                .as_com_ref::<IComponentHandler>()
                .expect("a handler")
                .as_ptr();
            assert_eq!(controller.setComponentHandler(recorder), kResultOk);
        }
        TestHost {
            blocks: Blocks::new(plugin.clone(), processor.clone()),
            component,
            plugin,
            processor,
            controller,
            handler,
            provoked: Cell::new(0),
        }
    }

    /// Returns how many faults the instance has had, on every thread.
    pub fn faults(&self) -> u64 {
        self.component.faults()
    }

    /// Declares `count` more faults the test provokes on any thread on
    /// purpose, which the check at drop then allows.
    pub fn expect_faults(&self, count: u64) {
        self.provoked.set(self.provoked.get() + count);
    }

    /// The component, for the exporter's own tests.
    #[cfg(test)]
    pub(crate) fn component(&self) -> &Component<P> {
        &self.component
    }

    /// The component's interfaces, for the exporter's own tests.
    #[cfg(test)]
    pub(crate) fn interfaces(
        &self,
    ) -> (
        &ComPtr<IComponent>,
        &ComPtr<IAudioProcessor>,
        &ComPtr<IEditController>,
    ) {
        (&self.plugin, &self.processor, &self.controller)
    }

    /// Returns how many parameters the host lists.
    pub fn parameter_count(&self) -> usize {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        usize::try_from(unsafe { self.controller.getParameterCount() }).unwrap_or(0)
    }

    /// Returns the parameter the host lists at `index`.
    pub fn parameter(&self, index: usize) -> Option<ParamInfo> {
        // SAFETY: all zeros is a valid info, which the call overwrites.
        let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
        let index = int32::try_from(index).ok()?;
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `info` is valid for writing one `ParameterInfo`
        // during the call.
        if unsafe { self.controller.getParameterInfo(index, &mut info) } != kResultOk {
            return None;
        }
        Some(ParamInfo {
            id: info.id,
            title: wide(&info.title),
            units: wide(&info.units),
            step_count: info.stepCount,
            default_normalized: info.defaultNormalizedValue,
            can_automate: info.flags & ParameterFlags_::kCanAutomate != 0,
            is_list: info.flags & ParameterFlags_::kIsList != 0,
            is_bypass: info.flags & ParameterFlags_::kIsBypass != 0,
        })
    }

    /// Returns the text the plugin shows for a 0..1 value.
    pub fn text(&self, id: u32, normalized: f64) -> Option<String> {
        let mut text: String128 = [0; 128];
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `text` is valid for writing one `String128` during
        // the call.
        let result = unsafe {
            self.controller
                .getParamStringByValue(id, normalized, &mut text)
        };
        (result == kResultOk).then(|| wide(&text))
    }

    /// Returns the 0..1 value the plugin reads from text.
    pub fn parse(&self, id: u32, text: &str) -> Option<f64> {
        let mut wide_text: String128 = [0; 128];
        copy_utf16(text, &mut wide_text);
        let mut normalized = 0.0;
        // SAFETY: the component is alive while this host holds its interfaces;
        // `wide_text` is a terminated `String128` and `normalized` is valid
        // for writing one value, both during the call.
        let result = unsafe {
            self.controller
                .getParamValueByString(id, wide_text.as_mut_ptr(), &mut normalized)
        };
        (result == kResultOk).then_some(normalized)
    }

    /// Returns the real value for the plugin's 0..1.
    pub fn plain(&self, id: u32, normalized: f64) -> f64 {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.controller.normalizedParamToPlain(id, normalized) }
    }

    /// Returns the plugin's 0..1 for a real value.
    pub fn normalize(&self, id: u32, plain: f64) -> f64 {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.controller.plainParamToNormalized(id, plain) }
    }

    /// Returns the parameter's current 0..1.
    pub fn normalized(&self, id: u32) -> f64 {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.controller.getParamNormalized(id) }
    }

    /// Sets a parameter as a host's own controller would, through
    /// `setParamNormalized`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer, for an id the plugin does not list say.
    pub fn set_normalized(&self, id: u32, normalized: f64) -> Result<(), Refused> {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        Refused::check(unsafe { self.controller.setParamNormalized(id, normalized) })
    }

    /// Asks for a main bus layout through `setBusArrangements`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer, for a layout the plugin does not declare, or
    /// while it is active.
    pub fn arrange(&self, layout: Layout) -> Result<(), Refused> {
        let mut input = arrangement(layout.inputs);
        let mut output = arrangement(layout.outputs);
        let input_count = int32::from(layout.inputs > 0);
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `input` and `output` are one arrangement each, as
        // the counts say.
        Refused::check(unsafe {
            self.processor
                .setBusArrangements(&mut input, input_count, &mut output, 1)
        })
    }

    /// Returns the main bus layout, read through `getBusArrangement`.
    pub fn layout(&self) -> Layout {
        let count = |direction| {
            let mut arrangement: SpeakerArrangement = 0;
            // SAFETY: the component is alive while this host holds its
            // interfaces, and `arrangement` is valid for writing one
            // arrangement.
            let result = unsafe {
                self.processor
                    .getBusArrangement(direction, 0, &mut arrangement)
            };
            if result == kResultOk {
                arrangement.count_ones() as usize
            } else {
                0
            }
        };
        Layout::new(
            count(BusDirections_::kInput as BusDirection),
            count(BusDirections_::kOutput as BusDirection),
        )
    }

    /// Calls `setupProcessing`, realtime and 32-bit.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer, for a rate that is not a positive number or
    /// while the plugin is active, say.
    pub fn setup(&self, sample_rate: f64, max_frames: usize) -> Result<(), Refused> {
        let mut setup = VstSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: int32::try_from(max_frames).unwrap_or(int32::MAX),
            sampleRate: sample_rate,
        };
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `setup` is valid for reading during the call.
        Refused::check(unsafe { self.processor.setupProcessing(&mut setup) })
    }

    /// Calls `setActive(true)`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer.
    pub fn activate(&self) -> Result<(), Refused> {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        Refused::check(unsafe { self.plugin.setActive(1) })
    }

    /// Calls `setActive(false)`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer.
    pub fn deactivate(&self) -> Result<(), Refused> {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        Refused::check(unsafe { self.plugin.setActive(0) })
    }

    /// Calls `setProcessing(true)`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer.
    pub fn start(&self) -> Result<(), Refused> {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        Refused::check(unsafe { self.processor.setProcessing(1) })
    }

    /// Calls `setProcessing(false)`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer.
    pub fn stop(&self) -> Result<(), Refused> {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        Refused::check(unsafe { self.processor.setProcessing(0) })
    }

    /// Processes `frames` frames from separate input buffers into separate
    /// output buffers, after `changes`.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        self.blocks.process(inputs, outputs, frames, changes)
    }

    /// Processes `frames` frames in place: the same buffers in and out, as
    /// many hosts hand them.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    pub fn process_in_place(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        self.blocks.process_in_place(channels, frames, changes)
    }

    /// Processes `frames` frames in place after `queues`, each a host id
    /// and its points as `(sample offset, 0..1)`, sent as the host sends
    /// automation.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::vst3::TestHost;
    ///
    /// let mut host = TestHost::<Gain>::new();
    /// let id = host.parameter(0).expect("listed").id;
    /// host.setup(48_000.0, 4)?;
    /// host.activate()?;
    /// host.start()?;
    /// let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
    /// // Two points in the block: the processor sees the last for the whole
    /// // block.
    /// let queues: [(u32, &[(i32, f64)]); 1] = [(id, &[(0, 0.5), (2, 0.0)])];
    /// let block = &mut [&mut left[..], &mut right[..]];
    /// host.process_queues(block, 4, &queues)?;
    /// let first = left[0];
    /// assert!((first - 0.0631).abs() < 1e-4, "-24 dB from the first sample");
    /// assert_eq!(host.normalized(id), 0.0);
    /// # Ok::<(), seam_host::vst3::Refused>(())
    /// ```
    pub fn process_queues(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        queues: &[(u32, &[(i32, f64)])],
    ) -> Result<(), Refused> {
        self.blocks.process_queues(channels, frames, queues)
    }

    /// Returns the output bus's silence flags after the last block: a bit per
    /// channel the plugin says is silent.
    pub fn silence_flags(&self) -> u64 {
        self.blocks.silence
    }

    /// Returns a side of this host for its audio thread, which processes
    /// blocks on a thread of its own while this one goes on.
    pub fn audio_side(&self) -> AudioSide<'_> {
        AudioSide {
            blocks: Blocks::new(self.plugin.clone(), self.processor.clone()),
            host: PhantomData,
        }
    }

    /// Returns the saved block, through `getState`.
    #[expect(
        clippy::missing_panics_doc,
        reason = "the stream's class lists `IBStream`, so asking it for one cannot fail"
    )]
    pub fn save(&self) -> Option<Vec<u8>> {
        let stream = ComWrapper::new(Stream::default());
        let pointer = stream.as_com_ref::<IBStream>().expect("a stream").as_ptr();
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `pointer` is a live `IBStream` until `stream`
        // drops, after the call.
        let result = unsafe { self.plugin.getState(pointer) };
        (result == kResultOk).then(|| stream.bytes.borrow().clone())
    }

    /// Loads a block through `setState`, then hands the controller the same
    /// block, as a host does.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer of the first of the two calls that refused,
    /// for a block that is not a saved one say.
    #[expect(
        clippy::missing_panics_doc,
        reason = "the stream's class lists `IBStream`, so asking it for one cannot fail"
    )]
    pub fn load(&self, bytes: &[u8]) -> Result<(), Refused> {
        let stream = ComWrapper::new(Stream::default());
        stream.bytes.borrow_mut().extend_from_slice(bytes);
        let pointer = stream.as_com_ref::<IBStream>().expect("a stream").as_ptr();
        // SAFETY: the component is alive while this host holds its
        // interfaces, and `pointer` is a live `IBStream` until `stream`
        // drops, after both calls.
        unsafe {
            Refused::check(self.plugin.setState(pointer))?;
            stream.position.set(0);
            Refused::check(self.controller.setComponentState(pointer))
        }
    }

    /// Runs one main-thread tick, as the exporter's timer runs it.
    pub fn tick(&self) {
        self.component.tick();
    }

    /// Returns what the plugin told the host since the last call.
    pub fn calls(&self) -> Vec<HostCall> {
        std::mem::take(&mut *self.handler.calls.borrow_mut())
    }

    /// Returns `getLatencySamples`.
    pub fn latency(&self) -> u32 {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.processor.getLatencySamples() }
    }

    /// Returns `getTailSamples`.
    pub fn tail(&self) -> u32 {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.processor.getTailSamples() }
    }
}

impl<P: Plugin> Drop for TestHost<P> {
    fn drop(&mut self) {
        // SAFETY: the component is alive while this host holds its
        // interfaces, and the call passes no pointer.
        unsafe { self.plugin.terminate() };
        if !std::thread::panicking() {
            assert_eq!(
                self.faults(),
                self.provoked.get(),
                "faults at the boundary that the test did not provoke"
            );
        }
    }
}

/// One side's way to send blocks: its own parameter changes and pointer
/// lists, so the host's main thread and its audio side each have theirs.
struct Blocks {
    plugin: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    changes: ComWrapper<Changes>,
    inputs: Vec<*mut f32>,
    outputs: Vec<*mut f32>,
    /// The output bus's silence flags after the last block.
    silence: u64,
}

impl Blocks {
    fn new(plugin: ComPtr<IComponent>, processor: ComPtr<IAudioProcessor>) -> Self {
        Blocks {
            plugin,
            processor,
            changes: ComWrapper::new(Changes::with_queues(64)),
            inputs: Vec::with_capacity(16),
            outputs: Vec::with_capacity(16),
            silence: 0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        assert!(inputs.iter().all(|channel| channel.len() >= frames));
        assert!(outputs.iter().all(|channel| channel.len() >= frames));
        self.inputs.clear();
        self.inputs
            .extend(inputs.iter().map(|channel| channel.as_ptr().cast_mut()));
        self.outputs.clear();
        self.outputs
            .extend(outputs.iter_mut().map(|channel| channel.as_mut_ptr()));
        self.fill(changes);
        self.send(frames, inputs.len())
    }

    fn process_in_place(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        let inputs = self.in_place(channels, frames);
        self.fill(changes);
        self.send(frames, inputs)
    }

    fn process_queues(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        queues: &[(u32, &[(i32, f64)])],
    ) -> Result<(), Refused> {
        let inputs = self.in_place(channels, frames);
        self.changes.fill(queues.iter().copied());
        self.send(frames, inputs)
    }

    /// Points both lists at `channels`, the inputs only as many as the
    /// input bus has, and answers how many that is.
    fn in_place(&mut self, channels: &mut [&mut [f32]], frames: usize) -> usize {
        assert!(channels.iter().all(|channel| channel.len() >= frames));
        self.outputs.clear();
        self.outputs
            .extend(channels.iter_mut().map(|channel| channel.as_mut_ptr()));
        self.inputs.clear();
        self.inputs.extend_from_slice(&self.outputs);
        let mut arrangement: SpeakerArrangement = 0;
        // SAFETY: the component is alive while this side holds its
        // interfaces, and `arrangement` is valid for writing one arrangement.
        let result = unsafe {
            self.processor.getBusArrangement(
                BusDirections_::kInput as BusDirection,
                0,
                &mut arrangement,
            )
        };
        let inputs = if result == kResultOk {
            arrangement.count_ones() as usize
        } else {
            0
        };
        let count = self.inputs.len().min(inputs);
        self.inputs.truncate(count);
        count
    }

    fn fill(&self, changes: &[(u32, f64)]) {
        self.changes.fill(
            changes
                .iter()
                .map(|(id, value)| Queued(*id, Values::Start(std::slice::from_ref(value)))),
        );
    }

    fn send(&mut self, frames: usize, input_channels: usize) -> Result<(), Refused> {
        // SAFETY: the component is alive while this side holds its
        // interfaces, and the call passes no pointer.
        let input_buses = unsafe {
            self.plugin.getBusCount(
                MediaTypes_::kAudio as int32,
                BusDirections_::kInput as int32,
            )
        };
        let mut input = AudioBusBuffers {
            numChannels: int32::try_from(input_channels).unwrap_or(0),
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: self.inputs.as_mut_ptr(),
            },
        };
        let mut output = AudioBusBuffers {
            numChannels: int32::try_from(self.outputs.len()).unwrap_or(0),
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: self.outputs.as_mut_ptr(),
            },
        };
        let changes = self
            .changes
            .as_com_ref::<IParameterChanges>()
            .expect("parameter changes")
            .as_ptr();
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            numSamples: int32::try_from(frames).unwrap_or(int32::MAX),
            numInputs: input_buses,
            numOutputs: 1,
            inputs: &mut input,
            outputs: &mut output,
            inputParameterChanges: changes,
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };
        // SAFETY: the component is alive while this side holds its
        // interfaces. `data` and its two buses live through the call. Each
        // bus's channel list holds `numChannels` pointers into buffers the
        // callers above borrow for at least `frames` floats. The changes and
        // their queues are this side's objects, alive throughout.
        let processed = unsafe { self.processor.process(&mut data) };
        self.silence = output.silenceFlags;
        Refused::check(processed)
    }
}

/// A test host's audio thread: it sends blocks, on a thread of its own,
/// while the host's main thread goes on. Made by
/// [`TestHost::audio_side`], and no longer-lived than the host.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::vst3::TestHost;
///
/// let host = TestHost::<Gain>::new();
/// let id = host.parameter(0).expect("listed").id;
/// host.setup(48_000.0, 4)?;
/// host.activate()?;
/// host.start()?;
///
/// // The host's audio thread gets a side of its own and sends blocks on it.
/// let mut audio = host.audio_side();
/// std::thread::scope(|scope| {
///     scope.spawn(move || {
///         let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
///         let block = &mut [&mut left[..], &mut right[..]];
///         assert_eq!(audio.process_in_place(block, 4, &[(id, 0.0)]), Ok(()));
///         assert!(left[0] < 0.1);
///     });
/// });
/// # Ok::<(), seam_host::vst3::Refused>(())
/// ```
pub struct AudioSide<'a> {
    blocks: Blocks,
    host: PhantomData<&'a ()>,
}

// SAFETY: its changes object and its queues are its own, reached through no
// other reference: the component sees them only during a `process` call
// this side makes. Moving the side therefore moves all access to them. Its
// pointer lists are refilled at each block and read only during it. Its
// interfaces are `ComPtr`s to a component that is `Send` and `Sync`.
unsafe impl Send for AudioSide<'_> {}

impl AudioSide<'_> {
    /// As [`TestHost::process`], on this side's thread.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        self.blocks.process(inputs, outputs, frames, changes)
    }

    /// As [`TestHost::process_in_place`], on this side's thread.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    pub fn process_in_place(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        changes: &[(u32, f64)],
    ) -> Result<(), Refused> {
        self.blocks.process_in_place(channels, frames, changes)
    }

    /// As [`TestHost::process_queues`], on this side's thread.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer to `process`.
    ///
    /// # Panics
    ///
    /// If a buffer is shorter than `frames`.
    pub fn process_queues(
        &mut self,
        channels: &mut [&mut [f32]],
        frames: usize,
        queues: &[(u32, &[(i32, f64)])],
    ) -> Result<(), Refused> {
        self.blocks.process_queues(channels, frames, queues)
    }

    /// Returns the output bus's silence flags after this side's last block.
    pub fn silence_flags(&self) -> u64 {
        self.blocks.silence
    }
}

impl std::fmt::Debug for AudioSide<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSide").finish_non_exhaustive()
    }
}

fn arrangement(channels: usize) -> SpeakerArrangement {
    match channels {
        0 => SpeakerArr::kEmpty,
        1 => SpeakerArr::kMono,
        2 => SpeakerArr::kStereo,
        _ => SpeakerArr::k51,
    }
}

fn wide(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

/// The host's component handler: records each call.
#[derive(Default)]
pub(super) struct Recorder {
    pub(super) calls: RefCell<Vec<HostCall>>,
}

impl Class for Recorder {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Recorder {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.calls.borrow_mut().push(HostCall::Begin(id));
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

/// The most points one of the host's queues holds without allocating.
const QUEUE_POINTS: usize = 64;

/// One parameter's points for a block.
struct Queue {
    id: Cell<u32>,
    points: RefCell<Vec<(i32, f64)>>,
}

impl Default for Queue {
    fn default() -> Self {
        Queue {
            id: Cell::new(0),
            points: RefCell::new(Vec::with_capacity(QUEUE_POINTS)),
        }
    }
}

/// A queue to send: a host id and its points, as `(offset, 0..1)`.
struct Queued<'a>(u32, Values<'a>);

/// A queue's points: given with offsets, or one value at the block's start.
enum Values<'a> {
    Placed(&'a [(i32, f64)]),
    Start(&'a [f64]),
}

impl<'a> From<(u32, &'a [(i32, f64)])> for Queued<'a> {
    fn from((id, points): (u32, &'a [(i32, f64)])) -> Self {
        Queued(id, Values::Placed(points))
    }
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        int32::try_from(self.points.borrow().len()).unwrap_or(int32::MAX)
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        let points = self.points.borrow();
        let point = usize::try_from(index)
            .ok()
            .and_then(|index| points.get(index));
        let (Some(&(at, normalized)), false, false) = (point, offset.is_null(), value.is_null())
        else {
            return kInvalidArgument;
        };
        // SAFETY: the exporter passes room for the point; both pointers are
        // non-null here.
        unsafe {
            *offset = at;
            *value = normalized;
        }
        kResultOk
    }

    unsafe fn addPoint(&self, _offset: int32, _value: ParamValue, _index: *mut int32) -> tresult {
        kResultFalse
    }
}

/// A block's parameter changes, one queue each.
struct Changes {
    queues: RefCell<Vec<ComWrapper<Queue>>>,
    count: Cell<usize>,
}

impl Changes {
    /// Room for `count` changes a block, so a block of up to that many
    /// allocates nothing.
    fn with_queues(count: usize) -> Self {
        Changes {
            queues: RefCell::new(
                (0..count)
                    .map(|_| ComWrapper::new(Queue::default()))
                    .collect(),
            ),
            count: Cell::new(0),
        }
    }

    fn fill<'a, S: Into<Queued<'a>>>(&self, sends: impl ExactSizeIterator<Item = S>) {
        let count = sends.len();
        let mut queues = self.queues.borrow_mut();
        while queues.len() < count {
            queues.push(ComWrapper::new(Queue::default()));
        }
        for (queue, send) in queues.iter().zip(sends) {
            let Queued(id, points) = send.into();
            queue.id.set(id);
            let mut held = queue.points.borrow_mut();
            held.clear();
            match points {
                Values::Placed(points) => held.extend_from_slice(points),
                Values::Start(values) => held.extend(values.iter().map(|&value| (0, value))),
            }
        }
        self.count.set(count);
    }
}

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> int32 {
        int32::try_from(self.count.get()).unwrap_or(int32::MAX)
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let queues = self.queues.borrow();
        usize::try_from(index)
            .ok()
            .filter(|&index| index < self.count.get())
            .and_then(|index| queues.get(index))
            .and_then(ComWrapper::as_com_ref::<IParamValueQueue>)
            .map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        _index: *mut int32,
    ) -> *mut IParamValueQueue {
        std::ptr::null_mut()
    }
}

/// A memory stream.
#[derive(Default)]
pub(super) struct Stream {
    pub(super) bytes: RefCell<Vec<u8>>,
    pub(super) position: Cell<usize>,
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
        let bytes = self.bytes.borrow();
        let start = self.position.get().min(bytes.len());
        let length = usize::try_from(count).unwrap_or(0).min(bytes.len() - start);
        if length > 0 && buffer.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the caller passes room for `count` bytes; `length` is at
        // most that.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes[start..].as_ptr(), buffer.cast::<u8>(), length);
        }
        self.position.set(start + length);
        if !read.is_null() {
            // SAFETY: the caller passes room for the count.
            unsafe { *read = length as int32 };
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, count: int32, written: *mut int32) -> tresult {
        let length = usize::try_from(count).unwrap_or(0);
        if length > 0 && buffer.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the caller passes `count` bytes.
        let source =
            unsafe { std::slice::from_raw_parts(buffer.cast::<u8>().cast_const(), length) };
        let mut bytes = self.bytes.borrow_mut();
        let start = self.position.get().min(bytes.len());
        bytes.truncate(start);
        bytes.extend_from_slice(source);
        self.position.set(start + length);
        if !written.is_null() {
            // SAFETY: the caller passes room for the count.
            unsafe { *written = length as int32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, position: int64, mode: int32, result: *mut int64) -> tresult {
        let length = self.bytes.borrow().len() as int64;
        let base = match mode {
            0 => 0,
            1 => self.position.get() as int64,
            2 => length,
            _ => return kInvalidArgument,
        };
        let target = (base + position).clamp(0, length);
        self.position.set(target as usize);
        if !result.is_null() {
            // SAFETY: the caller passes room for the position.
            unsafe { *result = target };
        }
        kResultOk
    }

    unsafe fn tell(&self, position: *mut int64) -> tresult {
        if position.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: as above.
        unsafe { *position = self.position.get() as int64 };
        kResultOk
    }
}
