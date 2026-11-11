# use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#
# use seam_host::{
#     AuCodes, Block, Category, EventList, Layout, Parts, Plugin, PluginInfo, ProcessSetup,
#     Processor, Status, Tail,
# };
# use seam_params::{Declarer, Float, FloatDecl, Params};
#
# /// Main-thread ticks the plugin has had.
# static TICKS: AtomicU32 = AtomicU32::new(0);
# /// Set to make the next tick panic.
# static BREAK: AtomicBool = AtomicBool::new(false);
# /// Set to make the next tick drag the gain to +6 dB, as the plugin's own UI
# /// would.
# static EDIT: AtomicBool = AtomicBool::new(false);
#
# /// One parameter, a gain in decibels.
# #[derive(Clone)]
# struct GainParams {
#     gain: Float,
# }
#
# impl Params for GainParams {
#     fn declare(p: &mut Declarer) -> Self {
#         GainParams {
#             gain: p.add(FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0).with_unit("dB")),
#         }
#     }
# }
#
# /// Scales every channel by the gain parameter, and sleeps on silence.
# struct Level(GainParams);
#
# impl Processor for Level {
#     fn process(&mut self, block: Block<'_, '_>) -> Status {
#         let gain = 10.0_f32.powf(self.0.gain.get() / 20.0);
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
# struct Gain {
#     parts: Parts,
#     params: GainParams,
# }
#
# impl Plugin for Gain {
#     const INFO: PluginInfo = PluginInfo {
#         name: "Gain",
#         vendor: "Example",
#         url: "",
#         email: "",
#         description: "",
#         version: "0.1.0",
#         id: "dev.example.gain",
#         vst3_class_id: [0; 16],
#         au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
#         category: Category::Effect,
#         features: &[],
#         layouts: &[Layout::STEREO],
#         midi_input: false,
#         midi_output: false,
#     };
#     type Params = GainParams;
#     type Processor = Level;
#
#     fn new(params: GainParams, parts: Parts) -> Self {
#         Gain { parts, params }
#     }
#
#     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
#         Level(self.params.clone())
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
#             self.parts.params().write(self.params.gain.id(), 6.0);
#         }
#     }
# }
