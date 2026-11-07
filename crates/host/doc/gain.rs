# use std::sync::Arc;
# use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#
# use seam_host::{
#     AuCodes, Block, Category, EventList, Layout, Plugin, PluginInfo, ProcessSetup, Processor,
#     SavedState, Status, Tail,
# };
# use seam_params::{AudioParams, Kind, ParamDecl, ParamId, ParamSet};
# use seam_session::Session;
#
# /// Main-thread ticks the plugin has had.
# static TICKS: AtomicU32 = AtomicU32::new(0);
# /// Set to make the next tick panic.
# static BREAK: AtomicBool = AtomicBool::new(false);
# /// Set to make the next tick drag the gain to +6 dB, as the plugin's own UI
# /// would.
# static EDIT: AtomicBool = AtomicBool::new(false);
#
# /// Scales every channel by the gain parameter, and sleeps on silence.
# struct Level {
#     params: AudioParams,
#     gain: ParamId,
# }
#
# impl Processor for Level {
#     fn process(&mut self, block: Block<'_, '_>) -> Status {
#         let gain = 10.0_f32.powf(self.params.value(self.gain) / 20.0);
#         let mut peak = 0.0_f32;
#         for channel in block.channels.iter_mut() {
#             for sample in channel.iter_mut() {
#                 *sample *= gain;
#                 peak = peak.max(sample.abs());
#             }
#         }
#         if peak == 0.0 { Status::Sleep } else { Status::Continue }
#     }
# }
#
# /// A plugin with one parameter, a gain in decibels.
# struct Gain(Arc<SavedState>);
#
# const GAIN_INFO: PluginInfo = PluginInfo {
#     name: "Gain",
#     vendor: "Example",
#     url: "",
#     email: "",
#     description: "",
#     version: "0.1.0",
#     id: "dev.example.gain",
#     vst3_class_id: [0; 16],
#     au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
#     category: Category::Effect,
#     features: &[],
#     layouts: &[Layout::STEREO],
#     midi_input: false,
#     midi_output: false,
# };
#
# const GAIN_PARAMS: &[ParamDecl] = &[
#     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0).with_unit("dB"),
# ];
#
# fn gain_new() -> Gain {
#     let set = ParamSet::from_decls(GAIN_PARAMS.iter().copied()).expect("valid declarations");
#     Gain(Arc::new(SavedState::new(&set, &Session::new(&set))))
# }
#
# fn gain_level(plugin: &Gain) -> Level {
#     let set = plugin.0.set();
#     Level { params: set.audio(), gain: set.id("gain").expect("declared") }
# }
#
# impl Plugin for Gain {
#     const INFO: PluginInfo = GAIN_INFO;
#     const PARAMS: &'static [ParamDecl] = GAIN_PARAMS;
#     type Processor = Level;
#
#     fn new() -> Self {
#         gain_new()
#     }
#
#     fn saved(&self) -> Arc<SavedState> {
#         Arc::clone(&self.0)
#     }
#
#     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
#         gain_level(self)
#     }
#
#     fn latency(&self) -> u32 {
#         64
#     }
#
#     fn tail(&self) -> Tail {
#         Tail::Frames(480)
#     }
#
#     fn idle(&mut self) {
#         assert!(!BREAK.load(Ordering::Relaxed), "the plugin broke");
#         TICKS.fetch_add(1, Ordering::Relaxed);
#         if EDIT.swap(false, Ordering::Relaxed) {
#             let set = self.0.set();
#             let ui = set.take_ui().expect("the first UI handle");
#             ui.write(set.id("gain").expect("declared"), 6.0);
#         }
#     }
# }
