# use seam_params::{Kind, ParamDecl, ParamSetBuilder};
# let mut builder = ParamSetBuilder::new();
# builder.add(ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 10.0, 1.0, None), 5.0)
#     .with_unit("dB"))?;
# let set = builder.build();
# let gain = set.id("gain").ok_or("declared above")?;
# let ui = set.take_ui().ok_or("the first UI handle")?;
