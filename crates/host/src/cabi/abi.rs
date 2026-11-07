//! The C interface's types and constants, each declared once. Every type
//! spells itself in C ([`CType`]), so the header [`header`](fn@super::header)
//! writes cannot disagree with the Rust it comes from.

use std::ffi::{c_char, c_void};

/// A struct as the header declares it.
#[derive(Debug)]
pub(super) struct CStruct {
    pub(super) name: &'static str,
    pub(super) doc: &'static [&'static str],
    pub(super) size: usize,
    pub(super) fields: Vec<CField>,
}

/// One field of a [`CStruct`].
#[derive(Debug)]
pub(super) struct CField {
    pub(super) name: &'static str,
    /// Its C declaration, spelled from its Rust type.
    pub(super) declaration: String,
    pub(super) doc: &'static [&'static str],
    pub(super) offset: usize,
}

/// A constant as the header defines it.
#[derive(Debug)]
pub(super) struct CConst {
    pub(super) name: &'static str,
    pub(super) value: u32,
    pub(super) doc: &'static [&'static str],
}

/// A type as C spells it.
pub(super) trait CType {
    /// A declaration of `name` with this type, or the type alone when
    /// `name` is empty. A function pointer names its parameters `args`,
    /// when there are any.
    fn declare(name: &str, args: &[&str]) -> String;
}

/// A function pointer's type, which `Option` may wrap as a nullable one.
pub(super) trait CFunction: CType {}

fn named(base: &str, name: &str) -> String {
    if name.is_empty() {
        base.to_owned()
    } else {
        format!("{base} {name}")
    }
}

macro_rules! c_scalars {
    ($($rust:ty => $c:literal),* $(,)?) => {
        $(
            impl CType for $rust {
                fn declare(name: &str, _: &[&str]) -> String {
                    named($c, name)
                }
            }
        )*
    };
}

c_scalars! {
    () => "void",
    c_void => "void",
    bool => "bool",
    CChar => "char",
    u8 => "uint8_t",
    u32 => "uint32_t",
    usize => "size_t",
    f32 => "float",
    f64 => "double",
}

impl<T: CType> CType for *const T {
    fn declare(name: &str, _: &[&str]) -> String {
        let pointee = T::declare("", &[]);
        let constant = if pointee.ends_with('*') {
            format!("{pointee} const")
        } else {
            format!("const {pointee}")
        };
        named(&format!("{constant}*"), name)
    }
}

impl<T: CType> CType for *mut T {
    fn declare(name: &str, _: &[&str]) -> String {
        named(&format!("{}*", T::declare("", &[])), name)
    }
}

impl<F: CFunction> CType for Option<F> {
    fn declare(name: &str, args: &[&str]) -> String {
        F::declare(name, args)
    }
}

macro_rules! c_functions {
    ($(($($arg:ident),*)),* $(,)?) => {
        $(
            impl<R: CType, $($arg: CType),*> CType for unsafe extern "C" fn($($arg),*) -> R {
                fn declare(name: &str, args: &[&str]) -> String {
                    let parameters: &[fn(&str) -> String] =
                        &[$(|name: &str| $arg::declare(name, &[])),*];
                    assert!(
                        args.is_empty() || args.len() == parameters.len(),
                        "{name}: {} names for {} parameters",
                        args.len(),
                        parameters.len()
                    );
                    let list: Vec<String> = parameters
                        .iter()
                        .enumerate()
                        .map(|(index, declare)| declare(args.get(index).copied().unwrap_or("")))
                        .collect();
                    let list = if list.is_empty() {
                        "void".to_owned()
                    } else {
                        list.join(", ")
                    };
                    format!("{} (*{name})({list})", R::declare("", &[]))
                }
            }

            impl<R: CType, $($arg: CType),*> CFunction for unsafe extern "C" fn($($arg),*) -> R {}
        )*
    };
}

c_functions! {
    (),
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
}

/// Declares `#[repr(C)]` structs and `structs()`, which lists them for the
/// header.
///
/// Each field's C declaration is spelled from its Rust type; a function
/// pointer's parameter names come from an `#[args(...)]` before it.
macro_rules! c_structs {
    ($(
        $(#[doc = $doc:expr])*
        pub struct $name:ident {
            $(
                $(#[doc = $field_doc:literal])*
                $(#[args($($arg:ident),* $(,)?)])?
                pub $field:ident: $rust:ty,
            )*
        }
    )*) => {
        $(
            $(#[doc = $doc])*
            #[repr(C)]
            #[derive(Debug, Clone, Copy)]
            pub struct $name {
                $(
                    $(#[doc = $field_doc])*
                    pub $field: $rust,
                )*
            }

            impl CType for $name {
                fn declare(name: &str, _: &[&str]) -> String {
                    named(stringify!($name), name)
                }
            }
        )*

        /// Every struct, in the order the header declares them.
        pub(super) fn structs() -> Vec<CStruct> {
            vec![$(
                CStruct {
                    name: stringify!($name),
                    doc: &[$($doc),*],
                    size: ::std::mem::size_of::<$name>(),
                    fields: vec![$(
                        CField {
                            name: stringify!($field),
                            declaration: <$rust as CType>::declare(
                                stringify!($field),
                                &[$($(stringify!($arg)),*)?],
                            ),
                            doc: &[$($field_doc),*],
                            offset: ::std::mem::offset_of!($name, $field),
                        },
                    )*],
                },
            )*]
        }
    };
}

/// Declares `u32` constants, each with its C name, and `consts()`, which
/// lists them for the header.
macro_rules! c_consts {
    ($(
        $(#[doc = $doc:literal])*
        pub const $name:ident = $value:expr => $c:literal;
    )*) => {
        $(
            $(#[doc = $doc])*
            pub const $name: u32 = $value;
        )*

        /// Every constant, in the order the header defines them.
        pub(super) fn consts() -> Vec<CConst> {
            vec![$(
                CConst {
                    name: $c,
                    value: $name,
                    doc: &[$($doc),*],
                },
            )*]
        }
    };
}

c_consts! {
    /// The interface's version: the adapter refuses a plugin built against
    /// another. The header and the plugin are built from one commit, so it
    /// moves only when the interface does.
    pub const ABI_VERSION = 1 => "SEAM_ABI_VERSION";
    /// A parameter whose value is a number in a range, continuous or
    /// stepped; a stepped range stays continuous to the host.
    pub const KIND_CONTINUOUS = 0 => "SEAM_KIND_CONTINUOUS";
    /// A parameter that is off or on.
    pub const KIND_BOOL = 1 => "SEAM_KIND_BOOL";
    /// A parameter that is one of a list of labels.
    pub const KIND_CHOICE = 2 => "SEAM_KIND_CHOICE";
    /// The parameter is the plugin's bypass, for the host to drive as such.
    pub const PARAM_BYPASS = 1 => "SEAM_PARAM_BYPASS";
    /// The host should re-read every parameter's value: a sync changed
    /// values without a gesture.
    pub const RESTART_VALUES = 1 => "SEAM_RESTART_VALUES";
    /// The latency changed; read it again with `latency`.
    pub const RESTART_LATENCY = 2 => "SEAM_RESTART_LATENCY";
    /// Keep processing, whatever the input.
    pub const STATUS_CONTINUE = 0 => "SEAM_STATUS_CONTINUE";
    /// Keep processing while the input or the output is not silent.
    pub const STATUS_CONTINUE_IF_NOT_QUIET = 1 => "SEAM_STATUS_CONTINUE_IF_NOT_QUIET";
    /// Keep processing for the tail once the input is silent.
    pub const STATUS_TAIL = 2 => "SEAM_STATUS_TAIL";
    /// Nothing to do until input or events arrive again.
    pub const STATUS_SLEEP = 3 => "SEAM_STATUS_SLEEP";
    /// The most frames `activate` builds for. A larger block size builds for
    /// this many, and one of 0 for 1,024 frames; a longer block plays in
    /// pieces.
    pub const MAX_FRAMES = crate::MAX_FRAMES as u32 => "SEAM_MAX_FRAMES";
    /// `tail` answers this when the output falls silent with the input.
    pub const TAIL_NONE = 0 => "SEAM_TAIL_NONE";
    /// `tail` answers this when the output may never fall silent.
    pub const TAIL_INFINITE = u32::MAX => "SEAM_TAIL_INFINITE";
}

/// A C `char`, whichever of `i8` and `u8` it is on the target, so its C
/// spelling is `char` everywhere.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::{CChar, plugin};
///
/// let table = plugin::<Gain>();
/// // SAFETY: a host's calls, all on this thread, with a buffer valid for
/// // the call.
/// unsafe {
///     let instance = (table.create)(&seam_host::cabi::SeamHost::NONE);
///     // The plugin writes the parameter's text into a buffer of `CChar`.
///     let mut buffer = [CChar(0); 16];
///     let whole = (table.text)(instance, 0, 0.0, buffer.as_mut_ptr(), buffer.len());
///     let text: Vec<u8> = buffer[..whole].iter().map(|c| c.0 as u8).collect();
///     assert_eq!(text, b"-24.00");
///     (table.destroy)(instance);
/// }
/// ```
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CChar(pub c_char);

/// One instance of the plugin, opaque to C.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use seam_host::cabi::{SeamHost, SeamInstance, plugin};
///
/// let table = plugin::<Gain>();
/// // SAFETY: a host's calls, all on this thread; the instance lives until
/// // `destroy`.
/// unsafe {
///     let instance: *mut SeamInstance = (table.create)(&SeamHost::NONE);
///     assert!(!instance.is_null());
///     assert_eq!((table.param_count)(instance), 1);
///     (table.destroy)(instance);
/// }
/// ```
#[repr(C)]
#[derive(Debug)]
pub struct SeamInstance {
    _opaque: [u8; 0],
}

impl CType for SeamInstance {
    fn declare(name: &str, _: &[&str]) -> String {
        named("SeamInstance", name)
    }
}

/// What the plugin hands `save`'s writer: the context, then the block.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use std::ffi::c_void;
///
/// use seam_host::cabi::{SeamHost, WriteFn, plugin};
///
/// /// The host's side of a save: keeps a copy of the block, which lives only
/// /// for the call.
/// unsafe extern "C" fn keep(
///     context: *mut c_void,
///     data: *const u8,
///     length: usize,
/// ) {
///     // SAFETY: the context is the `Vec` below, and `data` holds `length`
///     // bytes for the call.
///     unsafe {
///         let block = std::slice::from_raw_parts(data, length);
///         (*context.cast::<Vec<u8>>()).extend_from_slice(block);
///     }
/// }
///
/// let write: WriteFn = keep;
/// let mut saved: Vec<u8> = Vec::new();
/// let table = plugin::<Gain>();
/// // SAFETY: a host's calls, all on this thread; the writer and its context
/// // are valid for the call.
/// unsafe {
///     let instance = (table.create)(&SeamHost::NONE);
///     assert!((table.save)(instance, (&raw mut saved).cast(), Some(write)));
///     (table.destroy)(instance);
/// }
/// assert_eq!(saved.first(), Some(&b'{'), "one JSON object");
/// ```
pub type WriteFn = unsafe extern "C" fn(context: *mut c_void, data: *const u8, length: usize);

c_structs! {
    /// UTF-8 text, not terminated. The plugin's own points into its
    /// declarations and lives as long as it is loaded; the host's lives for
    /// the call it is passed to.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::cabi::{SeamHost, SeamStr, plugin};
    ///
    /// let table = plugin::<Gain>();
    /// // SAFETY: a host's calls, all on this thread; `normalized` is valid
    /// // for the call.
    /// unsafe {
    ///     let instance = (table.create)(&SeamHost::NONE);
    ///     let mut normalized = 0.0;
    ///     // A user types 6 into the host's field.
    ///     assert!((table.parse)(instance, 0, SeamStr::new("6"), &mut normalized));
    ///     assert_eq!(normalized, 0.625);
    ///     (table.destroy)(instance);
    /// }
    /// ```
    pub struct SeamStr {
        /// The first byte; any value when `length` is 0.
        pub data: *const CChar,
        /// How many bytes.
        pub length: usize,
    }

    /// A parameter, as the host lists it.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::cabi::{SeamHost, SeamParamInfo, SeamStr, plugin};
    ///
    /// let table = plugin::<Gain>();
    /// let empty = SeamStr::new("");
    /// let mut info = SeamParamInfo {
    ///     key: empty,
    ///     name: empty,
    ///     unit: empty,
    ///     group: empty,
    ///     host_id: 0,
    ///     kind: 0,
    ///     steps: 0,
    ///     flags: 0,
    ///     default_normalized: 0.0,
    /// };
    /// // SAFETY: a host's calls, all on this thread; `info` is valid for the
    /// // call, and the plugin's text lives while the library is loaded.
    /// unsafe {
    ///     let instance = (table.create)(&SeamHost::NONE);
    ///     assert!((table.param_info)(instance, 0, &mut info));
    ///     let key = std::slice::from_raw_parts(info.key.data.cast::<u8>(), info.key.length);
    ///     assert_eq!(key, b"gain");
    ///     assert_eq!(info.default_normalized, 0.5);
    ///     (table.destroy)(instance);
    /// }
    /// ```
    pub struct SeamParamInfo {
        /// Its key: the identity saved states and the page use. The JUCE
        /// adapter names the parameter to JUCE by it.
        pub key: SeamStr,
        /// The name a host shows.
        pub name: SeamStr,
        /// The unit shown after the value; empty for none.
        pub unit: SeamStr,
        /// The group path, `/`-separated; empty for the top level.
        pub group: SeamStr,
        /// The id Seam's own exporters give hosts, a hash of the key.
        pub host_id: u32,
        /// `SEAM_KIND_CONTINUOUS`, `SEAM_KIND_BOOL` or `SEAM_KIND_CHOICE`.
        pub kind: u32,
        /// How many values a bool or choice has; 0 for a continuous one.
        pub steps: u32,
        /// `SEAM_PARAM_BYPASS`, or 0.
        pub flags: u32,
        /// The default, as the host's 0..1.
        pub default_normalized: f64,
    }

    /// A main bus layout: channels in, channels out.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::cabi::{SeamLayout, plugin};
    ///
    /// let table = plugin::<Gain>();
    /// let mut layout = SeamLayout { inputs: 0, outputs: 0 };
    /// // SAFETY: `layout` is valid for the call, which takes no instance.
    /// unsafe {
    ///     assert_eq!((table.layout_count)(), 1);
    ///     assert!((table.layout)(0, &mut layout)); // the first is the default
    /// }
    /// assert_eq!((layout.inputs, layout.outputs), (2, 2));
    /// ```
    pub struct SeamLayout {
        /// Input channels; 0 for an instrument.
        pub inputs: u32,
        /// Output channels.
        pub outputs: u32,
    }

    /// One block of audio. A host processing in place passes the same
    /// buffers in both lists.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::cabi::{
    ///     STATUS_CONTINUE, SeamHost, SeamLayout, SeamProcess, plugin,
    /// };
    ///
    /// let table = plugin::<Gain>();
    /// let input = [[1.0_f32; 4]; 2];
    /// let (mut left, mut right) = ([0.0_f32; 4], [0.0_f32; 4]);
    /// let inputs = [input[0].as_ptr(), input[1].as_ptr()];
    /// let outputs = [left.as_mut_ptr(), right.as_mut_ptr()];
    /// let block = SeamProcess {
    ///     inputs: inputs.as_ptr(),
    ///     outputs: outputs.as_ptr(),
    ///     input_count: 2,
    ///     output_count: 2,
    ///     frames: 4,
    /// };
    /// // SAFETY: a host's calls, all on this thread; both lists hold two
    /// // channels of four frames that nothing else touches during the call.
    /// unsafe {
    ///     let instance = (table.create)(&SeamHost::NONE);
    ///     assert!((table.activate)(instance, 48_000.0, 4, SeamLayout { inputs: 2, outputs: 2 }));
    ///     assert_eq!((table.process)(instance, &block), STATUS_CONTINUE);
    ///     (table.destroy)(instance);
    /// }
    /// assert_eq!(left, [1.0; 4], "0 dB leaves the level alone");
    /// ```
    pub struct SeamProcess {
        /// `input_count` channels, each `frames` long; any may be null.
        pub inputs: *const *const f32,
        /// `output_count` channels, each `frames` long; any may be null.
        pub outputs: *const *mut f32,
        /// How many input channels.
        pub input_count: u32,
        /// How many output channels. Any past the plugin's outputs are
        /// zeroed.
        pub output_count: u32,
        /// The frames in this block. A block longer than the activation
        /// promised plays in pieces.
        pub frames: u32,
    }

    /// What the plugin calls back, only inside `idle` or `destroy`, on the
    /// thread that made that call. Any callback may be null. `context` and
    /// the callbacks stay valid until `destroy` returns.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use std::ffi::c_void;
    ///
    /// use seam_host::cabi::{SeamHost, plugin};
    ///
    /// /// The host's side of an edit: records the parameter index and 0..1
    /// /// value.
    /// unsafe extern "C" fn perform_edit(
    ///     context: *mut c_void,
    ///     index: u32,
    ///     normalized: f64,
    /// ) {
    ///     // SAFETY: the context is the `Vec` below, alive until `destroy` returns.
    ///     unsafe { (*context.cast::<Vec<(u32, f64)>>()).push((index, normalized)) };
    /// }
    ///
    /// let mut edits: Vec<(u32, f64)> = Vec::new();
    /// let host = SeamHost {
    ///     context: (&raw mut edits).cast(),
    ///     perform_edit: Some(perform_edit),
    ///     ..SeamHost::NONE // the other callbacks are null: the host ignores them
    /// };
    ///
    /// let table = plugin::<Gain>();
    /// // The plugin's UI drags the gain to +6 dB.
    /// EDIT.store(true, Ordering::Relaxed);
    /// // SAFETY: a host's calls, all on this thread; `host` is copied by
    /// // `create`.
    /// unsafe {
    ///     let instance = (table.create)(&host);
    ///     (table.idle)(instance); // the callbacks run inside the tick
    ///     (table.destroy)(instance);
    /// }
    /// assert_eq!(edits, [(0, 0.625)]);
    /// ```
    pub struct SeamHost {
        /// Handed back to every callback.
        pub context: *mut c_void,
        /// The user started changing the parameter at `index`.
        #[args(context, index)]
        pub begin_edit: Option<unsafe extern "C" fn(*mut c_void, u32)>,
        /// The parameter at `index` now holds `normalized`, already stored:
        /// tell the host, and do not write it back.
        #[args(context, index, normalized)]
        pub perform_edit: Option<unsafe extern "C" fn(*mut c_void, u32, f64)>,
        /// The user finished changing the parameter at `index`.
        #[args(context, index)]
        pub end_edit: Option<unsafe extern "C" fn(*mut c_void, u32)>,
        /// `SEAM_RESTART_VALUES` and `SEAM_RESTART_LATENCY`, or'd together.
        #[args(context, flags)]
        pub restart: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    }

    /// The plugin, as `seam_plugin` returns it: its facts and every call
    /// into it, each with the threads it may run on.
    ///
    /// A call marked "main thread" runs where the plugin's type says:
    ///
    /// - A plugin that cannot move between threads (one that is not
    ///   `Send`) keeps the thread that calls `create` as its main thread,
    ///   for good. Such a plugin refuses those calls from any other thread.
    /// - A plugin that can move has no main thread. Each such call, `idle`
    ///   and `destroy` included, takes the plugin on loan for its length,
    ///   from any thread, for the instance's whole life. One that arrives
    ///   while a call on another thread has it waits its turn, up to 50 ms,
    ///   and is refused past that. The bound keeps a host that holds a lock
    ///   of its own across both threads from deadlocking. Two do not wait:
    ///   an `idle` skips, counting nothing, and a `destroy` is refused, as
    ///   `destroy` says.
    ///
    /// A call made from inside another on the same thread shares its loan,
    /// as one from a host callback inside `idle` does. A refused call does
    /// nothing, answers as refused where it answers, and counts as a fault.
    ///
    /// Open: none of these calls is the audio thread's by contract. But
    /// JUCE's AAX wrapper calls `prepareToPlay` on the render thread for a
    /// longer block, so an `activate` that meets an `idle` waits there.
    ///
    /// No call unwinds: a panic inside one is caught and answered as a
    /// refusal. A block that panicked plays silence until the next
    /// `activate`.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../../doc/gain.rs")]
    /// use seam_host::cabi::{
    ///     STATUS_CONTINUE, SeamHost, SeamLayout, SeamProcess, plugin,
    /// };
    ///
    /// // A host's whole life with the plugin: create, activate, write a value,
    /// // process, destroy.
    /// let table = plugin::<Gain>();
    /// let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
    /// let channels = [left.as_mut_ptr(), right.as_mut_ptr()];
    /// let block = SeamProcess {
    ///     inputs: channels.as_ptr().cast(), // in place: the same buffers in both lists
    ///     outputs: channels.as_ptr(),
    ///     input_count: 2,
    ///     output_count: 2,
    ///     frames: 4,
    /// };
    /// // SAFETY: all calls on this thread; the lists hold two channels of four
    /// // frames that nothing else touches during the call.
    /// unsafe {
    ///     let instance = (table.create)(&SeamHost::NONE);
    ///     assert!((table.activate)(instance, 48_000.0, 4, SeamLayout { inputs: 2, outputs: 2 }));
    ///     (table.set)(instance, 0, 0.0); // -24 dB
    ///     assert_eq!((table.process)(instance, &block), STATUS_CONTINUE);
    ///     (table.destroy)(instance);
    /// }
    /// assert!(left[0] < 0.1);
    /// ```
    pub struct SeamPlugin {
        /// `SEAM_ABI_VERSION` as the plugin was built.
        pub abi_version: u32,
        /// How many main bus layouts it runs with; at least one. Any thread.
        pub layout_count: unsafe extern "C" fn() -> u32,
        /// The layout at `index`, the first the default. Any thread.
        #[args(index, layout)]
        pub layout: unsafe extern "C" fn(u32, *mut SeamLayout) -> bool,
        /// A new instance, or null. `host` is copied, and may be null. Any
        /// thread, which becomes the main thread of a plugin that cannot
        /// move between threads.
        #[args(host)]
        pub create: unsafe extern "C" fn(*const SeamHost) -> *mut SeamInstance,
        /// Deactivates and frees the instance, ending every gesture the host
        /// still has open.
        ///
        /// Main thread, with no other call into the instance running but the
        /// `idle` whose host callback makes this `destroy`. From such a
        /// callback the instance leaks, deactivated, and makes no further
        /// callback. A refused `destroy` leaks the instance too, and only its
        /// processor is dropped there. For a plugin that can move, a
        /// `destroy` is refused when it meets a call on another thread, which
        /// it does not wait for. That call goes on using the instance until it
        /// returns, and may make a callback after `destroy` does.
        #[args(instance)]
        pub destroy: unsafe extern "C" fn(*mut SeamInstance),
        /// How many parameters it has. Any thread.
        #[args(instance)]
        pub param_count: unsafe extern "C" fn(*mut SeamInstance) -> u32,
        /// The parameter at `index`, in the order hosts list them: depth
        /// first through the groups, each in declaration order. Any
        /// thread.
        #[args(instance, index, info)]
        pub param_info: unsafe extern "C" fn(*mut SeamInstance, u32, *mut SeamParamInfo) -> bool,
        /// The parameter's value, as 0..1. Any thread, the audio thread
        /// included.
        #[args(instance, index)]
        pub get: unsafe extern "C" fn(*mut SeamInstance, u32) -> f64,
        /// A host write of 0..1: the host's automation or a control.
        /// Any thread, the audio thread included.
        #[args(instance, index, normalized)]
        pub set: unsafe extern "C" fn(*mut SeamInstance, u32, f64),
        /// The text for `normalized`, written to `text` with a terminator, cut
        /// at a character to fit `capacity`.
        ///
        /// Answers the whole text's length, so a caller can tell it was cut.
        /// With no room (a null `text` or a `capacity` of 0), it writes nothing
        /// and answers the length, so a caller can size a buffer. Any thread
        /// but audio.
        #[args(instance, index, normalized, text, capacity)]
        pub text: unsafe extern "C" fn(*mut SeamInstance, u32, f64, *mut CChar, usize) -> usize,
        /// The 0..1 value `text` means, into `normalized`; false when it
        /// means none. Any thread but audio.
        #[args(instance, index, text, normalized)]
        pub parse: unsafe extern "C" fn(*mut SeamInstance, u32, SeamStr, *mut f64) -> bool,
        /// Builds the processor for a sample rate, a block size and one of the
        /// layouts, replacing any earlier one.
        ///
        /// A block size past `SEAM_MAX_FRAMES` builds for that many. `latency`
        /// and `tail` answer for it on return. Main thread.
        #[args(instance, sample_rate, max_frames, layout)]
        pub activate: unsafe extern "C" fn(*mut SeamInstance, f64, u32, SeamLayout) -> bool,
        /// Takes the processor back. Main thread.
        #[args(instance)]
        pub deactivate: unsafe extern "C" fn(*mut SeamInstance),
        /// The main thread's tick, about 60 times a second from `create` to
        /// `destroy`. The host callbacks run inside it; a tick called
        /// from inside one of them does nothing. Main thread.
        #[args(instance)]
        pub idle: unsafe extern "C" fn(*mut SeamInstance),
        /// The processing delay in frames, as last read. Any thread.
        #[args(instance)]
        pub latency: unsafe extern "C" fn(*mut SeamInstance) -> u32,
        /// The tail in frames, `SEAM_TAIL_NONE` or `SEAM_TAIL_INFINITE`, as
        /// last read. Any thread.
        #[args(instance)]
        pub tail: unsafe extern "C" fn(*mut SeamInstance) -> u32,
        /// Hands the saved block to `write`, once. Any thread but audio.
        #[args(instance, context, write)]
        pub save: unsafe extern "C" fn(*mut SeamInstance, *mut c_void, Option<WriteFn>) -> bool,
        /// Loads a saved block; false when it is not one, which changes
        /// nothing, as a block longer than 64 MiB is not. Any thread but
        /// audio.
        #[args(instance, data, length)]
        pub load: unsafe extern "C" fn(*mut SeamInstance, *const u8, usize) -> bool,
        /// Clears what the processor carries between blocks, before the
        /// next one. Any thread.
        #[args(instance)]
        pub reset: unsafe extern "C" fn(*mut SeamInstance),
        /// Processes one block; answers a `SEAM_STATUS_` value. Audio
        /// thread: never allocates, locks or waits.
        #[args(instance, block)]
        pub process: unsafe extern "C" fn(*mut SeamInstance, *const SeamProcess) -> u32,
    }
}

impl SeamStr {
    /// Lends `text` to the C interface as a string view.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_host::cabi::SeamStr;
    ///
    /// let key = SeamStr::new("gain");
    /// assert_eq!(key.length, 4);
    /// // SAFETY: the view points into the `'static` text it was made from.
    /// let bytes = unsafe { std::slice::from_raw_parts(key.data.cast::<u8>(), key.length) };
    /// assert_eq!(bytes, b"gain");
    /// ```
    pub fn new(text: &'static str) -> Self {
        SeamStr {
            data: text.as_ptr().cast(),
            length: text.len(),
        }
    }
}

impl SeamHost {
    /// No callbacks: a host that listens to nothing.
    pub const NONE: SeamHost = SeamHost {
        context: std::ptr::null_mut(),
        begin_edit: None,
        perform_edit: None,
        end_edit: None,
        restart: None,
    };
}
