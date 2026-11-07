# The comparison's C++ side: the example EQ written straight in JUCE, as a
# VST3 (A); the Rust EQ through Seam's C interface and the generic JUCE
# adapter, as a VST3 (C); and bench-parity, which answers parity.py's cases.
# See README.md.
cmake_minimum_required(VERSION 3.25)

# The same C runtime as Rust's MSVC builds, whose sinf, cosf, tanf and powf
# the Rust EQ calls: the dynamic one.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")

project(seam_bench VERSION 0.1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

# JUCE 9.0.0. A local checkout of it can stand in for the network.
set(SEAM_BENCH_JUCE_GIT "https://github.com/juce-framework/JUCE.git"
    CACHE STRING "Where to clone JUCE 9.0.0 from: its URL, or a local checkout that has the commit")
FetchContent_Declare(juce
    GIT_REPOSITORY ${SEAM_BENCH_JUCE_GIT}
    GIT_TAG f8f8864172464b9adf9eba6101e1f784838d1597)
FetchContent_MakeAvailable(juce)

# Floating point the way the comparison needs it (bench/README.md): precise,
# which in Visual Studio 2022 contracts nothing into fused multiply-adds, on x64,
# and in clang-cl contracts only where the target has them, which x64 does not.
if(MSVC)
    add_compile_options(/fp:precise)
endif()

juce_add_plugin(BenchJuceEq
    COMPANY_NAME "Seam"
    PLUGIN_MANUFACTURER_CODE Seam
    PLUGIN_CODE BjEq
    FORMATS VST3
    PRODUCT_NAME "Bench JUCE EQ"
    IS_SYNTH FALSE
    NEEDS_MIDI_INPUT FALSE
    NEEDS_MIDI_OUTPUT FALSE
    IS_MIDI_EFFECT FALSE
    COPY_PLUGIN_AFTER_BUILD FALSE
    VST3_CATEGORIES Fx EQ)

target_sources(BenchJuceEq PRIVATE juce-eq/PluginProcessor.cpp)
target_compile_definitions(BenchJuceEq PUBLIC
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_VST3_CAN_REPLACE_VST2=0)
target_link_libraries(BenchJuceEq
    PRIVATE juce::juce_audio_utils juce::juce_dsp
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)

# A with JUCE's own ranges: the same EQ as a JUCE developer would declare its
# parameters, for timing only. It stores values a little apart from Seam's,
# so it is outside the bit-for-bit check (README, "A with JUCE's ranges").
juce_add_plugin(BenchJuceEqNative
    COMPANY_NAME "Seam"
    PLUGIN_MANUFACTURER_CODE Seam
    PLUGIN_CODE BjEn
    FORMATS VST3
    PRODUCT_NAME "Bench JUCE EQ native"
    IS_SYNTH FALSE
    NEEDS_MIDI_INPUT FALSE
    NEEDS_MIDI_OUTPUT FALSE
    IS_MIDI_EFFECT FALSE
    COPY_PLUGIN_AFTER_BUILD FALSE
    VST3_CATEGORIES Fx EQ)

target_sources(BenchJuceEqNative PRIVATE juce-eq/PluginProcessor.cpp)
target_compile_definitions(BenchJuceEqNative PUBLIC
    BENCH_JUCE_NATIVE_RANGES=1
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_VST3_CAN_REPLACE_VST2=0)
target_link_libraries(BenchJuceEqNative
    PRIVATE juce::juce_audio_utils juce::juce_dsp
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)

# A-port: A-native with the Rust EQ's filters written in C++ (juce-eq/EqPort.h)
# in place of JUCE's dsp::IIR, for timing only (README, "The ladder"). A
# folder configured with clang-cl builds the same source under its own name.
if(CMAKE_CXX_COMPILER_ID STREQUAL "Clang")
    set(bench_port_name "Bench JUCE EQ port clang")
    set(bench_port_code BjEc)
else()
    set(bench_port_name "Bench JUCE EQ port")
    set(bench_port_code BjEp)
endif()
juce_add_plugin(BenchJuceEqPort
    COMPANY_NAME "Seam"
    PLUGIN_MANUFACTURER_CODE Seam
    PLUGIN_CODE ${bench_port_code}
    FORMATS VST3
    PRODUCT_NAME "${bench_port_name}"
    IS_SYNTH FALSE
    NEEDS_MIDI_INPUT FALSE
    NEEDS_MIDI_OUTPUT FALSE
    IS_MIDI_EFFECT FALSE
    COPY_PLUGIN_AFTER_BUILD FALSE
    VST3_CATEGORIES Fx EQ)

target_sources(BenchJuceEqPort PRIVATE juce-eq/PluginProcessor.cpp)
target_compile_definitions(BenchJuceEqPort PUBLIC
    BENCH_JUCE_NATIVE_RANGES=1
    BENCH_PORTED_DSP=1
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_VST3_CAN_REPLACE_VST2=0)
target_link_libraries(BenchJuceEqPort
    PRIVATE juce::juce_audio_utils juce::juce_dsp
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)

# C: the Rust EQ as JUCE builds it through the adapter, under a name of its
# own so a host lists it apart from Seam's native VST3 (B).
include(../adapters/juce/SeamJuce.cmake)
seam_add_juce_plugin(BenchSeamJuceEq
    PACKAGE seam-example-eq
    SETTINGS "${CMAKE_CURRENT_LIST_DIR}/../examples/eq/seam-juce.cmake"
    PRODUCT_NAME "Seam Example EQ JUCE"
    FORMATS VST3)

# bench-hosted, the console host: loads one .vst3 through JUCE's VST3
# hosting, for scripts/hosted.py and scripts/compare.py.
juce_add_console_app(BenchHosted PRODUCT_NAME "bench-hosted")
target_sources(BenchHosted PRIVATE hosted/hosted.cpp)
target_compile_definitions(BenchHosted PRIVATE
    JUCE_PLUGINHOST_VST3=1
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0)
target_link_libraries(BenchHosted
    PRIVATE juce::juce_audio_processors avrt psapi
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_warning_flags)

juce_add_console_app(BenchParity PRODUCT_NAME "bench-parity")
target_sources(BenchParity PRIVATE parity/parity.cpp juce-eq/PluginProcessor.cpp)
target_compile_definitions(BenchParity PRIVATE
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JucePlugin_Name="Bench JUCE EQ")
target_link_libraries(BenchParity
    PRIVATE juce::juce_audio_utils juce::juce_dsp
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)

# bench-parity on A-port's filters with Seam's ranges, so parity.py checks
# that the port computes what the Rust EQ does.
juce_add_console_app(BenchParityPort PRODUCT_NAME "bench-parity-port")
target_sources(BenchParityPort PRIVATE parity/parity.cpp juce-eq/PluginProcessor.cpp)
target_compile_definitions(BenchParityPort PRIVATE
    BENCH_PORTED_DSP=1
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JucePlugin_Name="Bench JUCE EQ")
target_link_libraries(BenchParityPort
    PRIVATE juce::juce_audio_utils juce::juce_dsp
    PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)
