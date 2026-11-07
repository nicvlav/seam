# use std::cell::RefCell;
# use std::rc::Rc;
# use seam_params::ParamSetBuilder;
# use seam_session::{ApplyError, Participant, Section, Session};
# use serde_json::json;
# struct Zoom(f64);
# impl Participant for Zoom {
#     fn capture(&self) -> Section {
#         json!(self.0)
#     }
#     fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
#         self.0 = section.and_then(Section::as_f64).unwrap_or(1.0);
#         Ok(())
#     }
# }
