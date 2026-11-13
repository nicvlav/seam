# use std::error::Error;
#
# use seam_host::{
#     AuCodes, Block, Category, Editor, Layout, ParentWindow, Parts, Plugin, PluginInfo,
#     ProcessSetup, Processor, Size, Status,
# };
#
# /// An editor that draws nothing and never touches its window.
# struct Blank;
#
# impl Editor for Blank {
#     fn size(&self) -> Size {
#         Size { width: 320, height: 200 }
#     }
#
#     fn open(&mut self, _parent: &ParentWindow) -> Result<(), Box<dyn Error>> {
#         Ok(())
#     }
#
#     fn close(&mut self) {}
#     fn set_size(&mut self, _size: Size) {}
#     fn set_scale(&mut self, _scale: f64) {}
#     fn tick(&mut self) {}
# }
#
# /// Passes its input through.
# struct Through;
#
# impl Processor for Through {
#     fn process(&mut self, _block: Block<'_, '_>) -> Status {
#         Status::Continue
#     }
# }
#
# /// A plugin with no parameter, and an editor of 320 by 200.
# struct Panel;
#
# impl Plugin for Panel {
#     const INFO: PluginInfo = PluginInfo {
#         name: "Panel",
#         vendor: "Example",
#         url: "",
#         email: "",
#         description: "",
#         version: "0.1.0",
#         id: "dev.example.panel",
#         vst3_class_id: [1; 16],
#         au_codes: AuCodes { subtype: *b"panl", manufacturer: *b"Exmp" },
#         category: Category::Effect,
#         features: &[],
#         layouts: &[Layout::STEREO],
#         midi_input: false,
#         midi_output: false,
#     };
#     type Params = ();
#     type Processor = Through;
#
#     fn new(_params: (), _parts: Parts) -> Self {
#         Panel
#     }
#
#     fn activate(&mut self, _setup: &ProcessSetup) -> Through {
#         Through
#     }
#
#     fn editor(&mut self) -> Option<Box<dyn Editor>> {
#         Some(Box::new(Blank))
#     }
# }
