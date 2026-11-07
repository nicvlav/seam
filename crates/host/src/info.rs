//! What a host reads about a plugin before creating it.

/// What kind of plugin it is.
///
/// # Examples
///
/// ```
/// use seam_host::{Category, Layout};
///
/// // An instrument has no input.
/// let (category, layout) = (Category::Instrument, Layout::new(0, 2));
/// assert_eq!((category, layout.inputs), (Category::Instrument, 0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Processes audio.
    Effect,
    /// Makes sound from notes.
    Instrument,
}

/// What a plugin does, finer than its [`Category`]: each exporter lists it
/// in its format's own words, as VST3's subcategories, and as CLAP's
/// features once the planned CLAP exporter is built. More join as plugins
/// need them.
///
/// # Examples
///
/// ```
/// use seam_host::{Category, Feature};
///
/// // What the example EQ tells a host's plugin browser.
/// const CATEGORY: Category = Category::Effect;
/// const FEATURES: &[Feature] = &[Feature::Equalizer, Feature::Filter];
/// assert_eq!(FEATURES.len(), 2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Feature {
    /// Measures or displays the signal.
    Analyzer,
    /// Echoes.
    Delay,
    /// Saturation, overdrive, distortion.
    Distortion,
    /// Compressors, limiters, gates, expanders.
    Dynamics,
    /// An equalizer.
    Equalizer,
    /// A filter.
    Filter,
    /// For the master bus.
    Mastering,
    /// Chorus, flanger, phaser, tremolo.
    Modulation,
    /// Changes pitch.
    PitchShift,
    /// Removes noise, clicks or hum.
    Restoration,
    /// Reverb.
    Reverb,
    /// Places sound in space.
    Spatial,
    /// A utility: gain, routing, metering helpers.
    Tools,
    /// Drum sounds.
    Drum,
    /// A piano.
    Piano,
    /// Plays samples.
    Sampler,
    /// A synthesizer.
    Synthesizer,
}

/// The channels of the main bus, in and out.
///
/// The VST3 exporter offers a bus of one channel as mono and of two as
/// stereo, and refuses to build a plugin that declares a wider one. The C
/// interface takes any width. An instrument has no input: `inputs` is 0.
///
/// # Examples
///
/// ```
/// use seam_host::Layout;
///
/// // A plugin that runs as mono or as stereo; the first is the default.
/// const LAYOUTS: &[Layout] = &[Layout::STEREO, Layout::MONO];
/// assert_eq!(LAYOUTS[0], Layout::new(2, 2));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Input channels.
    pub inputs: usize,
    /// Output channels.
    pub outputs: usize,
}

impl Layout {
    /// One channel in, one out.
    pub const MONO: Layout = Layout::new(1, 1);
    /// Two channels in, two out.
    pub const STEREO: Layout = Layout::new(2, 2);

    /// Creates a layout of `inputs` channels in and `outputs` out.
    pub const fn new(inputs: usize, outputs: usize) -> Self {
        Layout { inputs, outputs }
    }
}

/// A plugin's identity and description, the same for every format.
///
/// A plugin's [`Plugin::INFO`](crate::Plugin::INFO) is one of these, a
/// constant.
///
/// # Examples
///
/// ```
/// use seam_host::{Category, Feature, Layout, PluginInfo};
///
/// const INFO: PluginInfo = PluginInfo {
///     name: "Tone",
///     vendor: "Example",
///     url: "https://example.com",
///     email: "",
///     description: "A two-band equalizer.",
///     version: "0.1.0",
///     id: "com.example.tone",
///     // A version 4 UUID, made once for this plugin and never changed.
///     vst3_class_id: 0x8f3c_2a71_5d0e_4b96_a1c4_7e25_d9b0_3f68_u128.to_be_bytes(),
///     au_codes: (*b"tone", *b"Exmp"),
///     category: Category::Effect,
///     features: &[Feature::Equalizer],
///     layouts: &[Layout::STEREO, Layout::MONO],
///     midi_input: true,
///     midi_output: false,
/// };
/// assert_eq!(INFO.layouts[0], Layout::STEREO);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginInfo {
    /// The name hosts list.
    pub name: &'static str,
    /// Who makes it.
    pub vendor: &'static str,
    /// The vendor's web address; empty for none.
    pub url: &'static str,
    /// The vendor's contact address; empty for none.
    pub email: &'static str,
    /// One line on what it does; empty for none.
    pub description: &'static str,
    /// Its version, `major.minor.patch`.
    pub version: &'static str,
    /// A reverse-DNS id, unique to this plugin (`com.vendor.product`): the
    /// JUCE adapter's bundle id, and the id a CLAP build will take. No other
    /// format's id derives from it.
    pub id: &'static str,
    /// The VST3 class id: 16 bytes no other plugin uses.
    ///
    /// Make it once from 16 random bytes, such as a version 4 UUID's, and
    /// never change it after release: a host treats a plugin with a new id as
    /// a different plugin, and its sessions lose it.
    pub vst3_class_id: [u8; 16],
    /// The AU subtype and manufacturer codes, four characters each. The JUCE
    /// adapter's build also takes them as its plugin and manufacturer codes,
    /// from which JUCE derives that build's VST3 class id.
    pub au_codes: ([u8; 4], [u8; 4]),
    /// What kind of plugin it is.
    pub category: Category,
    /// What it does, finer: for a host's browser.
    pub features: &'static [Feature],
    /// The main bus layouts it runs with, the first by default; at least
    /// one. A host picks one before activation, and
    /// [`ProcessSetup`](crate::ProcessSetup) reports it.
    pub layouts: &'static [Layout],
    /// Whether it takes MIDI input (for MIDI learn, or notes).
    ///
    /// Open: no exporter carries MIDI either way yet. The VST3 exporter shows
    /// a host an event bus for each direction a plugin declares, but a
    /// block's [`events`](crate::Block::events) stay empty, and what a
    /// processor pushes to [`out_events`](crate::Block::out_events) reaches
    /// no host.
    pub midi_input: bool,
    /// Whether it sends MIDI (a MIDI effect, or MIDI-CI replies), through
    /// [`Block::out_events`](crate::Block::out_events).
    ///
    /// Open: no exporter sends it yet, as [`midi_input`](Self::midi_input)
    /// says.
    pub midi_output: bool,
}

impl PluginInfo {
    /// Its VST3 subcategories, in the SDK's words: its category, then each
    /// feature ("Fx", "EQ"). The VST3 exporter joins them with `|`, and the
    /// JUCE adapter's build hands them to JUCE, which does the same.
    #[cfg(any(feature = "vst3", feature = "cabi"))]
    pub(crate) fn vst3_categories(&self) -> Vec<&'static str> {
        let first = match self.category {
            Category::Effect => "Fx",
            Category::Instrument => "Instrument",
        };
        std::iter::once(first)
            .chain(
                self.features
                    .iter()
                    .map(|&feature| vst3_subcategory(feature, self.category)),
            )
            .collect()
    }
}

/// The VST3 subcategory of a feature, in a plugin of `category`.
#[cfg(any(feature = "vst3", feature = "cabi"))]
fn vst3_subcategory(feature: Feature, category: Category) -> &'static str {
    match feature {
        Feature::Analyzer => "Analyzer",
        Feature::Delay => "Delay",
        Feature::Distortion => "Distortion",
        Feature::Dynamics => "Dynamics",
        Feature::Equalizer => "EQ",
        Feature::Filter => "Filter",
        Feature::Mastering => "Mastering",
        Feature::Modulation => "Modulation",
        Feature::PitchShift => "Pitch Shift",
        Feature::Restoration => "Restoration",
        Feature::Reverb => "Reverb",
        Feature::Spatial => "Spatial",
        Feature::Tools => "Tools",
        // The SDK's names differ: `Fx|Drums` and `Instrument|Drum`.
        Feature::Drum if category == Category::Effect => "Drums",
        Feature::Drum => "Drum",
        Feature::Piano => "Piano",
        Feature::Sampler => "Sampler",
        Feature::Synthesizer => "Synth",
    }
}

#[cfg(all(test, any(feature = "vst3", feature = "cabi")))]
mod tests {
    use super::*;

    #[test]
    fn drums_are_named_as_the_sdk_names_them() {
        assert_eq!(vst3_subcategory(Feature::Drum, Category::Effect), "Drums");
        assert_eq!(
            vst3_subcategory(Feature::Drum, Category::Instrument),
            "Drum"
        );
        assert_eq!(vst3_subcategory(Feature::Equalizer, Category::Effect), "EQ");
    }
}
