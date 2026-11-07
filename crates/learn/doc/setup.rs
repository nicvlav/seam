# use seam_params::{Kind, ParamDecl, ParamSetBuilder};
# let mut builder = ParamSetBuilder::new();
# builder.add(ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 1.0, 0.0, None), 0.0))?;
# let set = builder.build();
# let gain = set.id("gain").ok_or("declared above")?;
# use seam_ump::{ChannelVoice, EventList, Voice};
# // One block of MIDI 1.0 control changes on group 0 and channel 0, as
# // (controller, value) pairs, as the `Learn` example builds it.
# let block = |changes: &[(u8, u8)]| {
#     let mut events = EventList::with_capacity(16);
#     for &(controller, value) in changes {
#         let voice = Voice::Midi1ControlChange { controller, value };
#         events.push(ChannelVoice { group: 0, channel: 0, voice }.event(0));
#     }
#     events
# };
# // A target that does nothing, for the examples that only register one:
# // the `Target` example shows what a target does.
# struct Fader;
# impl seam_learn::Target for Fader {
#     fn begin(&mut self) {}
#     fn apply(&mut self, _normalized: f64) {}
#     fn end(&mut self) {}
# }
