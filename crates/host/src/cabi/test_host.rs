//! A host inside the test's own process, driving a plugin through the C
//! interface's table as the JUCE adapter does.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;

use super::abi::{SeamHost, SeamInstance, SeamLayout, SeamParamInfo, SeamPlugin, SeamProcess};
use super::{CChar, SeamStr, plugin, plugin_moving};
use crate::{Layout, Plugin};

/// What the plugin told the host through its callbacks.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::{HostCall, TestHost};
///
/// let host = TestHost::<Gain>::new();
///
/// // The plugin's UI drags the gain to +6 dB.
/// EDIT.store(true, Ordering::Relaxed);
/// host.idle();
/// // The host hears the whole gesture, by parameter index, the value as 0..1.
/// assert_eq!(
///     host.calls(),
///     [HostCall::Begin(0), HostCall::Perform(0, 0.625), HostCall::End(0)]
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostCall {
    /// `begin_edit`, with the parameter's index.
    Begin(u32),
    /// `perform_edit`: the index and the 0..1 value.
    Perform(u32, f64),
    /// `end_edit`.
    End(u32),
    /// `restart`, with its flags.
    Restart(u32),
}

/// A parameter as the host lists it.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::{KIND_CONTINUOUS, TestHost};
///
/// let host = TestHost::<Gain>::new();
/// let info = host.parameter(0).expect("listed");
/// let names = (info.key.as_str(), info.name.as_str(), info.unit.as_str());
/// assert_eq!(names, ("gain", "Gain", "dB"));
/// assert_eq!((info.kind, info.steps), (KIND_CONTINUOUS, 0));
/// assert_eq!(info.default_normalized, 0.5, "0 dB, halfway through -24..24");
/// assert_eq!(info.host_id, seam_params::host_id("gain"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    /// Its key.
    pub key: String,
    /// Its name.
    pub name: String,
    /// Its unit.
    pub unit: String,
    /// Its group path.
    pub group: String,
    /// The id Seam's own exporters give hosts.
    pub host_id: u32,
    /// A `KIND_` constant.
    pub kind: u32,
    /// The values of a bool or choice; 0 for a continuous one.
    pub steps: u32,
    /// `PARAM_BYPASS`, or 0.
    pub flags: u32,
    /// The default, as 0..1.
    pub default_normalized: f64,
}

/// A host of the C interface for tests, holding one instance of `P`.
///
/// Made on the test's thread, which becomes the instance's main thread.
/// Every call goes through the plugin's table, as the adapter makes them; an
/// instance [created elsewhere](Self::created_elsewhere) gets the table that
/// binds its plugin to no thread. Processing takes the host's buffers either
/// separate or in place; a block of up to 16 channels allocates nothing in
/// the host.
///
/// Its instance must have no fault, on any thread, before it is destroyed:
/// it checks when dropped, allowing only the faults a test provokes on
/// purpose and declares with [`expect_faults`](Self::expect_faults).
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::TestHost;
///
/// let host = TestHost::<Gain>::new();
/// assert!(host.activate(48_000.0, 4, Layout::STEREO));
///
/// host.set(0, 0.0); // the host writes -24 dB
/// let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
/// let status = host.process_in_place(&mut [&mut left[..], &mut right[..]], 4);
/// assert_eq!(status, seam_host::cabi::STATUS_CONTINUE);
/// assert!((left[0] - 0.0631).abs() < 1e-4);
/// ```
pub struct TestHost<P: Plugin> {
    table: &'static SeamPlugin,
    instance: *mut SeamInstance,
    /// What the callbacks recorded; the host's context points here. An
    /// `Rc`, so moving the host moves no claim on the recorder that would
    /// invalidate the instance's pointer to it, as moving a `Box` would.
    calls: Rc<RefCell<Vec<HostCall>>>,
    provoked: Cell<u64>,
    plugin: PhantomData<fn() -> P>,
}

/// The most channels a block of the test host carries without allocating.
const MAX_CHANNELS: usize = 16;

impl<P: Plugin> TestHost<P> {
    /// Creates the instance, with callbacks that record what it tells the
    /// host.
    ///
    /// # Panics
    ///
    /// When `create` answers null.
    pub fn new() -> Self {
        Self::create(plugin::<P>(), false)
    }

    /// Creates the instance on another thread, as a host may, from the
    /// table that binds its plugin to no thread. This thread may then make
    /// its main-thread calls, and so may any other.
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    pub fn created_elsewhere() -> Self
    where
        P: Send,
    {
        Self::create(plugin_moving::<P>(), true)
    }

    /// Creates the instance from `table`, on another thread when
    /// `elsewhere`.
    pub(super) fn create(table: &'static SeamPlugin, elsewhere: bool) -> Self {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let host = SeamHost {
            context: Rc::as_ptr(&calls).cast_mut().cast::<c_void>(),
            begin_edit: Some(begin_edit),
            perform_edit: Some(perform_edit),
            end_edit: Some(end_edit),
            restart: Some(restart),
        };
        let host = Carried(host);
        let create = move || {
            let host = host;
            // SAFETY: `host` is readable for this call, which copies it. Its
            // callbacks are this module's, which never unwind. Its context
            // is the recorder, which outlives the instance: the host holds
            // it, and `release` destroys the instance before the host drops
            // it.
            let instance = unsafe { (table.create)(&host.0) };
            Carried(instance)
        };
        let Carried(instance) = if elsewhere {
            std::thread::scope(|scope| scope.spawn(create).join())
                .expect("created on another thread")
        } else {
            create()
        };
        assert!(!instance.is_null(), "create answered null");
        TestHost {
            table,
            instance,
            calls,
            provoked: Cell::new(0),
            plugin: PhantomData,
        }
    }

    /// Returns the table, for a test that calls it directly.
    pub fn table(&self) -> &'static SeamPlugin {
        self.table
    }

    /// Returns the instance, for a test that calls the table directly.
    pub fn instance(&self) -> *mut SeamInstance {
        self.instance
    }

    /// Returns how many faults the instance has had, on every thread; none
    /// once it is destroyed.
    pub fn faults(&self) -> u64 {
        // SAFETY: `self.instance` is null once destroyed, or the live
        // instance `create` made: only `release` frees it, and that takes the
        // host by `&mut`.
        unsafe { super::instance::<P>(self.instance) }.map_or(0, super::Instance::<P>::faults)
    }

    /// Declares that the test provokes `count` more faults on purpose: a
    /// panic or a call on the wrong thread.
    pub fn expect_faults(&self, count: u64) {
        self.provoked.set(self.provoked.get() + count);
    }

    /// Returns the main bus layouts the plugin lists.
    pub fn layouts(&self) -> Vec<Layout> {
        // SAFETY: `layout_count` takes no pointer.
        let count = unsafe { (self.table.layout_count)() };
        (0..count)
            .filter_map(|index| {
                let mut layout = SeamLayout {
                    inputs: 0,
                    outputs: 0,
                };
                // SAFETY: `layout` is valid for writing one `SeamLayout`
                // during the call.
                unsafe { (self.table.layout)(index, &mut layout) }
                    .then(|| Layout::new(layout.inputs as usize, layout.outputs as usize))
            })
            .collect()
    }

    /// Returns how many parameters the instance lists.
    pub fn parameter_count(&self) -> usize {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.param_count)(self.instance) as usize }
    }

    /// Returns the parameter at `index`.
    pub fn parameter(&self, index: u32) -> Option<ParamInfo> {
        let empty = SeamStr {
            data: std::ptr::null(),
            length: 0,
        };
        let mut info = SeamParamInfo {
            key: empty,
            name: empty,
            unit: empty,
            group: empty,
            host_id: 0,
            kind: 0,
            steps: 0,
            flags: 0,
            default_normalized: 0.0,
        };
        // SAFETY: `self.instance` is the live instance `new` made (only
        // `release`, which takes the host by `&mut`, frees it), and `info` is
        // valid for writing one `SeamParamInfo` during the call.
        if !unsafe { (self.table.param_info)(self.instance, index, &mut info) } {
            return None;
        }
        Some(ParamInfo {
            key: text_of(info.key),
            name: text_of(info.name),
            unit: text_of(info.unit),
            group: text_of(info.group),
            host_id: info.host_id,
            kind: info.kind,
            steps: info.steps,
            flags: info.flags,
            default_normalized: info.default_normalized,
        })
    }

    /// Returns the parameter's value, as 0..1.
    pub fn get(&self, index: u32) -> f64 {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.get)(self.instance, index) }
    }

    /// Writes a host's 0..1 value.
    pub fn set(&self, index: u32, normalized: f64) {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.set)(self.instance, index, normalized) }
    }

    /// Returns the text for `normalized`, in a buffer of `capacity` bytes, and
    /// the whole text's length, which is more when it was cut.
    ///
    /// # Panics
    ///
    /// If the instance writes an unterminated or non-UTF-8 text, or more than
    /// the whole length it answered.
    pub fn text(&self, index: u32, normalized: f64, capacity: usize) -> (String, usize) {
        let mut buffer = vec![0u8; capacity];
        // SAFETY: `self.instance` is the live instance `new` made (only
        // `release`, which takes the host by `&mut`, frees it), and `buffer`
        // is valid for writing `capacity` bytes during the call.
        let whole = unsafe {
            (self.table.text)(
                self.instance,
                index,
                normalized,
                buffer.as_mut_ptr().cast::<CChar>(),
                capacity,
            )
        };
        let length = buffer.iter().position(|&byte| byte == 0).unwrap_or(0);
        assert!(capacity == 0 || length < capacity, "terminated");
        assert!(length <= whole, "no more than the whole text");
        buffer.truncate(length);
        (String::from_utf8(buffer).expect("UTF-8"), whole)
    }

    /// Returns the 0..1 value `text` means, or `None` when it means none.
    pub fn parse(&self, index: u32, text: &str) -> Option<f64> {
        let mut normalized = f64::NAN;
        let text = SeamStr {
            data: text.as_ptr().cast(),
            length: text.len(),
        };
        // SAFETY: `self.instance` is the live instance `new` made (only
        // `release`, which takes the host by `&mut`, frees it). `text` points
        // into a `&str` that outlives the call, and `normalized` is valid for
        // writing one `f64` during it.
        unsafe { (self.table.parse)(self.instance, index, text, &mut normalized) }
            .then_some(normalized)
    }

    /// Activates for a sample rate, a block size and a layout, and returns
    /// whether it did.
    pub fn activate(&self, sample_rate: f64, max_frames: u32, layout: Layout) -> bool {
        let channels = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
        let layout = SeamLayout {
            inputs: channels(layout.inputs),
            outputs: channels(layout.outputs),
        };
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.activate)(self.instance, sample_rate, max_frames, layout) }
    }

    /// Deactivates the instance.
    pub fn deactivate(&self) {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.deactivate)(self.instance) }
    }

    /// Runs one main-thread tick.
    pub fn idle(&self) {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.idle)(self.instance) }
    }

    /// Returns the latency the instance answers.
    pub fn latency(&self) -> u32 {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.latency)(self.instance) }
    }

    /// Returns the tail the instance answers.
    pub fn tail(&self) -> u32 {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.tail)(self.instance) }
    }

    /// Returns the saved block, or `None` when `save` refused.
    pub fn save(&self) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        // SAFETY: `self.instance` is the live instance `new` made (only
        // `release`, which takes the host by `&mut`, frees it). `collect`
        // reads its context as the `Vec<u8>` it is, `bytes`, which outlives
        // the call, and never unwinds.
        let saved = unsafe {
            (self.table.save)(
                self.instance,
                std::ptr::from_mut(&mut bytes).cast::<c_void>(),
                Some(collect),
            )
        };
        saved.then_some(bytes)
    }

    /// Loads a saved block and returns whether it was recognised.
    pub fn load(&self, bytes: &[u8]) -> bool {
        // SAFETY: `self.instance` is the live instance `new` made (only
        // `release`, which takes the host by `&mut`, frees it), and `bytes`
        // is valid for reading its length during the call.
        unsafe { (self.table.load)(self.instance, bytes.as_ptr(), bytes.len()) }
    }

    /// Asks the next block to reset the processor.
    pub fn reset(&self) {
        // SAFETY: `self.instance` is the live instance `new` made: only
        // `release` frees it, and that takes the host by `&mut`.
        unsafe { (self.table.reset)(self.instance) }
    }

    /// Processes one block with separate input and output buffers, each at
    /// least `frames` long, and returns the status.
    ///
    /// # Panics
    ///
    /// With more than 16 channels either way, or a buffer shorter than
    /// `frames`.
    pub fn process(&self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) -> u32 {
        self.audio().process(inputs, outputs, frames)
    }

    /// Processes one block in place, as JUCE hands it: the same buffers for
    /// input and output.
    ///
    /// # Panics
    ///
    /// As [`process`](Self::process).
    pub fn process_in_place(&self, channels: &mut [&mut [f32]], frames: usize) -> u32 {
        self.audio().process_in_place(channels, frames)
    }

    /// Returns a side of this host for its audio thread, which processes
    /// blocks and writes values on a thread of its own while this one goes on.
    pub fn audio_side(&self) -> AudioSide<'_> {
        self.audio()
    }

    fn audio(&self) -> AudioSide<'_> {
        AudioSide {
            table: self.table,
            instance: self.instance,
            host: PhantomData,
        }
    }

    /// Returns what the plugin told the host since the last call, oldest first.
    pub fn calls(&self) -> Vec<HostCall> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    /// Destroys the instance now and returns what it told the host as it
    /// went: the ends of the gestures still open.
    pub fn destroy(mut self) -> Vec<HostCall> {
        self.release();
        self.calls()
    }

    fn release(&mut self) {
        if !self.instance.is_null() {
            // SAFETY: `self.instance` is non-null, so the live instance `new`
            // made; it is destroyed once, since the pointer is cleared after.
            // `&mut self` means no call through this host is in flight, and
            // `TestHost` is not `Send`, so this runs on the instance's main
            // thread.
            unsafe { (self.table.destroy)(self.instance) };
            self.instance = std::ptr::null_mut();
        }
    }
}

impl<P: Plugin> Default for TestHost<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Plugin> Drop for TestHost<P> {
    fn drop(&mut self) {
        let faults = self.faults();
        self.release();
        if !std::thread::panicking() {
            assert_eq!(
                faults,
                self.provoked.get(),
                "the instance's calls recorded faults the test did not provoke"
            );
        }
    }
}

impl<P: Plugin> std::fmt::Debug for TestHost<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHost")
            .field("plugin", &P::INFO.name)
            .finish_non_exhaustive()
    }
}

/// A test host's audio thread: it processes blocks and writes values, on a
/// thread of its own, while the host's main thread goes on. Made by
/// [`TestHost::audio_side`], and no longer-lived than the host.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::TestHost;
///
/// let host = TestHost::<Gain>::new();
/// assert!(host.activate(48_000.0, 4, Layout::STEREO));
///
/// // The host's audio thread gets a side of its own and sends blocks on it.
/// let audio = host.audio_side();
/// std::thread::scope(|scope| {
///     scope.spawn(move || {
///         audio.set(0, 0.0);
///         let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
///         audio.process_in_place(&mut [&mut left[..], &mut right[..]], 4);
///         assert!(left[0] < 0.1);
///     });
/// });
/// ```
pub struct AudioSide<'a> {
    table: &'static SeamPlugin,
    instance: *mut SeamInstance,
    host: PhantomData<&'a ()>,
}

// SAFETY: the instance pointer is the host's live instance: the side borrows
// the host, which frees it only when dropped. The calls this side makes on
// it, `process` and `set`, are ones the header allows on any thread.
unsafe impl Send for AudioSide<'_> {}

impl AudioSide<'_> {
    /// As [`TestHost::process`], on this side's thread.
    ///
    /// # Panics
    ///
    /// With more than 16 channels either way, or a buffer shorter than
    /// `frames`.
    pub fn process(&self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) -> u32 {
        assert!(inputs.len() <= MAX_CHANNELS && outputs.len() <= MAX_CHANNELS);
        assert!(inputs.iter().all(|channel| channel.len() >= frames));
        assert!(outputs.iter().all(|channel| channel.len() >= frames));
        let mut input_list = [std::ptr::null::<f32>(); MAX_CHANNELS];
        for (slot, channel) in input_list.iter_mut().zip(inputs) {
            *slot = channel.as_ptr();
        }
        let mut output_list = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
        for (slot, channel) in output_list.iter_mut().zip(outputs.iter_mut()) {
            *slot = channel.as_mut_ptr();
        }
        self.run(
            input_list.as_ptr(),
            inputs.len(),
            output_list.as_ptr(),
            outputs.len(),
            frames,
        )
    }

    /// As [`TestHost::process_in_place`], on this side's thread.
    ///
    /// # Panics
    ///
    /// As [`process`](Self::process).
    pub fn process_in_place(&self, channels: &mut [&mut [f32]], frames: usize) -> u32 {
        assert!(channels.len() <= MAX_CHANNELS);
        assert!(channels.iter().all(|channel| channel.len() >= frames));
        let mut list = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
        for (slot, channel) in list.iter_mut().zip(channels.iter_mut()) {
            *slot = channel.as_mut_ptr();
        }
        self.run(
            list.as_ptr().cast::<*const f32>(),
            channels.len(),
            list.as_ptr(),
            channels.len(),
            frames,
        )
    }

    /// Writes a host's 0..1 value, as JUCE's wrappers do on the audio
    /// thread.
    pub fn set(&self, index: u32, normalized: f64) {
        // SAFETY: `self.instance` is the host's live instance (see `Send`
        // above), and `set` takes no pointer.
        unsafe { (self.table.set)(self.instance, index, normalized) }
    }

    fn run(
        &self,
        inputs: *const *const f32,
        input_count: usize,
        outputs: *const *mut f32,
        output_count: usize,
        frames: usize,
    ) -> u32 {
        let block = SeamProcess {
            inputs,
            outputs,
            input_count: input_count as u32,
            output_count: output_count as u32,
            frames: u32::try_from(frames).expect("frames fit"),
        };
        // SAFETY: `self.instance` is the host's live instance (see `Send`
        // above). The callers above fill each list with the given count of
        // pointers into buffers of at least `frames` floats. They borrow the
        // buffers for this call, mutably for the outputs, so nothing else
        // touches them. In place, one buffer is in both lists, which the
        // header allows.
        unsafe { (self.table.process)(self.instance, &block) }
    }
}

impl std::fmt::Debug for AudioSide<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSide").finish_non_exhaustive()
    }
}

fn text_of(text: SeamStr) -> String {
    if text.length == 0 {
        return String::new();
    }
    // SAFETY: a `SeamStr` the plugin answers points into its declarations:
    // `length` bytes, readable while the library is loaded (the header's
    // contract), and `length` is not 0 here.
    let bytes = unsafe { std::slice::from_raw_parts(text.data.cast::<u8>(), text.length) };
    String::from_utf8(bytes.to_vec()).expect("UTF-8")
}

/// The host's callbacks on their way to the thread that creates the
/// instance, and the instance on its way back.
struct Carried<T>(T);

// SAFETY: only `SeamHost` and an instance pointer are carried, and both
// cross unread. The instance calls the callbacks only inside an `idle` or a
// `destroy`, on the thread making that call. The instance is `Sync`, its
// table refusing a main-thread call from a thread that may not make it.
unsafe impl<T> Send for Carried<T> {}

/// The recorder behind a host's context.
///
/// # Safety
///
/// `context` points to the `RefCell<Vec<HostCall>>` a live test host holds,
/// and this runs on that host's thread.
unsafe fn record(context: *mut c_void, call: HostCall) {
    // SAFETY: the caller's contract; the `RefCell` refuses an overlapping
    // borrow at run time.
    let calls = unsafe { &*context.cast::<RefCell<Vec<HostCall>>>() };
    calls.borrow_mut().push(call);
}

unsafe extern "C" fn begin_edit(context: *mut c_void, index: u32) {
    // SAFETY: the instance hands back the context this host gave, its
    // recorder, which lives as long as the instance, and calls back only on
    // the main thread, the host's own.
    unsafe { record(context, HostCall::Begin(index)) };
}

unsafe extern "C" fn perform_edit(context: *mut c_void, index: u32, normalized: f64) {
    // SAFETY: as in `begin_edit`: the context is this host's recorder.
    unsafe { record(context, HostCall::Perform(index, normalized)) };
}

unsafe extern "C" fn end_edit(context: *mut c_void, index: u32) {
    // SAFETY: as in `begin_edit`: the context is this host's recorder.
    unsafe { record(context, HostCall::End(index)) };
}

unsafe extern "C" fn restart(context: *mut c_void, flags: u32) {
    // SAFETY: as in `begin_edit`: the context is this host's recorder.
    unsafe { record(context, HostCall::Restart(flags)) };
}

/// `save`'s writer, appending to the `Vec` behind `context`.
unsafe extern "C" fn collect(context: *mut c_void, data: *const u8, length: usize) {
    // SAFETY: `context` is the `Vec<u8>` `save` passed, alive and borrowed
    // by nothing else during the call, and the header makes `data` valid for
    // reading `length` bytes until this returns.
    unsafe {
        let bytes = &mut *context.cast::<Vec<u8>>();
        if length > 0 {
            bytes.extend_from_slice(std::slice::from_raw_parts(data, length));
        }
    }
}
