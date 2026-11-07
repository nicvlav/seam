# The generic JUCE adapter's build: turns a Seam plugin crate into a JUCE
# plugin target, with no product C++.
#
#   seam_add_juce_plugin(<target>
#       PACKAGE <the plugin's cargo package>
#       SETTINGS <its generated seam-juce.cmake>
#       FORMATS <JUCE formats...>
#       [MANIFEST_PATH <the Cargo.toml to build from; Seam's workspace by default>]
#       [LIB_NAME <the crate's library name; the package's, with - as _, by default>]
#       [PRODUCT_NAME <a name other than the plugin's>])
#
# 1. It reads the plugin's settings, which `cargo xtask gen` writes from the
#    plugin's PluginInfo: names, codes, categories, MIDI.
# 2. It builds the package as a static library with only its `cabi` feature,
#    in a cargo target folder of its own under the build tree, so the native
#    exporters' entry points stay out of JUCE's binary. Cargo runs on every
#    build, does nothing when nothing changed, and a Rust change relinks the
#    plugin. On macOS and iOS it builds one library per architecture in
#    CMAKE_OSX_ARCHITECTURES (the host's alone when that is empty), each with
#    `cargo --target`, for CMAKE_OSX_DEPLOYMENT_TARGET when it is set, and
#    joins them with lipo, so a universal plugin links a universal library;
#    each architecture's Rust target must be installed (`rustup target add
#    aarch64-apple-darwin x86_64-apple-darwin`; for iOS `aarch64-apple-ios`,
#    and for its simulator `aarch64-apple-ios-sim` and `x86_64-apple-ios`).
# 3. It adds the JUCE plugin, the adapter's sources (SeamPluginProcessor) and
#    seam.h, and links the library.
#
# seam.h is the one committed beside the seam-host this file comes with, so
# the plugin must build against that same seam-host; the adapter checks
# SEAM_ABI_VERSION at run time all the same. JUCE must be added first. The
# Rust side is always built with cargo's release profile; it takes the C
# runtime from whatever the JUCE target links, so Debug and Release builds of
# the plugin both work.

set(SEAM_JUCE_ADAPTER_DIR "${CMAKE_CURRENT_LIST_DIR}")
get_filename_component(SEAM_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

# The commands that build the library once per Apple architecture and join
# the archives into one, into `out_build`, and that archive's path into
# `out_archive`.
function(_seam_apple_build out_build out_archive cargo_dir lib_name)
    set(cargo ${ARGN})
    set(archs ${CMAKE_OSX_ARCHITECTURES})
    if(NOT archs)
        set(archs ${CMAKE_SYSTEM_PROCESSOR})
    endif()
    if(CMAKE_SYSTEM_NAME STREQUAL "iOS")
        set(os ios)
    else()
        set(os darwin)
    endif()
    # Rust's arm64 simulator target is its own; its x86_64 iOS target is the
    # simulator's already.
    set(arm64_suffix)
    string(TOLOWER "${CMAKE_OSX_SYSROOT}" sysroot)
    if(os STREQUAL "ios" AND sysroot MATCHES "simulator")
        set(arm64_suffix -sim)
    endif()
    set(env)
    if(CMAKE_OSX_DEPLOYMENT_TARGET)
        if(os STREQUAL "ios")
            set(env "${CMAKE_COMMAND}" -E env "IPHONEOS_DEPLOYMENT_TARGET=${CMAKE_OSX_DEPLOYMENT_TARGET}")
        else()
            set(env "${CMAKE_COMMAND}" -E env "MACOSX_DEPLOYMENT_TARGET=${CMAKE_OSX_DEPLOYMENT_TARGET}")
        endif()
    endif()
    set(build)
    set(archives)
    foreach(arch IN LISTS archs)
        if(arch STREQUAL "arm64" OR arch STREQUAL "aarch64")
            set(triple "aarch64-apple-${os}${arm64_suffix}")
        elseif(arch STREQUAL "x86_64")
            set(triple "x86_64-apple-${os}")
        else()
            message(FATAL_ERROR "seam_add_juce_plugin: no Rust target for the Apple architecture ${arch}")
        endif()
        list(APPEND build COMMAND ${env} ${cargo} --target ${triple})
        list(APPEND archives "${cargo_dir}/${triple}/release/lib${lib_name}.a")
    endforeach()
    set(archive "${cargo_dir}/universal/lib${lib_name}.a")
    find_program(SEAM_LIPO lipo REQUIRED)
    list(APPEND build
        COMMAND "${CMAKE_COMMAND}" -E make_directory "${cargo_dir}/universal"
        COMMAND "${SEAM_LIPO}" -create -output "${archive}" ${archives})
    set(${out_build} ${build} PARENT_SCOPE)
    set(${out_archive} "${archive}" PARENT_SCOPE)
endfunction()

function(seam_add_juce_plugin target)
    cmake_parse_arguments(PARSE_ARGV 1 SEAM ""
        "PACKAGE;SETTINGS;MANIFEST_PATH;LIB_NAME;PRODUCT_NAME" "FORMATS")
    if(NOT SEAM_PACKAGE OR NOT SEAM_SETTINGS OR NOT SEAM_FORMATS)
        message(FATAL_ERROR "seam_add_juce_plugin(${target}) needs PACKAGE, SETTINGS and FORMATS")
    endif()
    include("${SEAM_SETTINGS}")
    if(NOT SEAM_PRODUCT_NAME)
        set(SEAM_PRODUCT_NAME "${SEAM_PLUGIN_NAME}")
    endif()
    if(NOT SEAM_MANIFEST_PATH)
        set(SEAM_MANIFEST_PATH "${SEAM_ROOT}/Cargo.toml")
    endif()
    if(NOT SEAM_LIB_NAME)
        string(REPLACE "-" "_" SEAM_LIB_NAME "${SEAM_PACKAGE}")
    endif()

    find_program(SEAM_CARGO cargo REQUIRED)
    set(cargo_dir "${CMAKE_BINARY_DIR}/seam-cargo")
    set(cargo "${SEAM_CARGO}" rustc --manifest-path "${SEAM_MANIFEST_PATH}"
        --package "${SEAM_PACKAGE}" --release --locked
        --no-default-features --features cabi --crate-type staticlib
        --target-dir "${cargo_dir}")
    set(build)
    if(APPLE)
        set(system)
        _seam_apple_build(build archive "${cargo_dir}" "${SEAM_LIB_NAME}" ${cargo})
    elseif(WIN32)
        set(archive "${cargo_dir}/release/${SEAM_LIB_NAME}.lib")
        # What rustc lists as the library's native dependencies
        # (`--print native-static-libs`).
        set(system kernel32 ntdll userenv ws2_32 dbghelp)
        set(build COMMAND ${cargo})
    else()
        set(archive "${cargo_dir}/release/lib${SEAM_LIB_NAME}.a")
        set(system pthread dl m)
        set(build COMMAND ${cargo})
    endif()

    # Beside the manifest, so rustup takes the toolchain its project pins.
    get_filename_component(manifest_dir "${SEAM_MANIFEST_PATH}" DIRECTORY)
    add_custom_target(${target}_rust
        ${build}
        WORKING_DIRECTORY "${manifest_dir}"
        BYPRODUCTS "${archive}"
        COMMENT "Building ${SEAM_PACKAGE} as a static library"
        USES_TERMINAL
        VERBATIM)
    add_library(${target}_seam STATIC IMPORTED)
    set_target_properties(${target}_seam PROPERTIES IMPORTED_LOCATION "${archive}")
    target_link_libraries(${target}_seam INTERFACE ${system})

    juce_add_plugin(${target}
        PRODUCT_NAME "${SEAM_PRODUCT_NAME}"
        COMPANY_NAME "${SEAM_PLUGIN_VENDOR}"
        COMPANY_WEBSITE "${SEAM_PLUGIN_URL}"
        COMPANY_EMAIL "${SEAM_PLUGIN_EMAIL}"
        DESCRIPTION "${SEAM_PLUGIN_DESCRIPTION}"
        VERSION "${SEAM_PLUGIN_VERSION}"
        BUNDLE_ID "${SEAM_PLUGIN_ID}"
        PLUGIN_MANUFACTURER_CODE "${SEAM_PLUGIN_MANUFACTURER_CODE}"
        PLUGIN_CODE "${SEAM_PLUGIN_CODE}"
        FORMATS ${SEAM_FORMATS}
        IS_SYNTH ${SEAM_PLUGIN_IS_SYNTH}
        NEEDS_MIDI_INPUT ${SEAM_PLUGIN_MIDI_INPUT}
        NEEDS_MIDI_OUTPUT ${SEAM_PLUGIN_MIDI_OUTPUT}
        IS_MIDI_EFFECT FALSE
        COPY_PLUGIN_AFTER_BUILD FALSE
        VST3_CATEGORIES ${SEAM_PLUGIN_VST3_CATEGORIES})

    target_sources(${target} PRIVATE "${SEAM_JUCE_ADAPTER_DIR}/SeamPluginProcessor.cpp")
    target_include_directories(${target} PRIVATE
        "${SEAM_JUCE_ADAPTER_DIR}"
        "${SEAM_ROOT}/crates/host/include")
    target_compile_definitions(${target} PUBLIC
        JUCE_WEB_BROWSER=0
        JUCE_USE_CURL=0
        JUCE_VST3_CAN_REPLACE_VST2=0
        JUCE_DISPLAY_SPLASH_SCREEN=0)
    target_link_libraries(${target}
        PRIVATE ${target}_seam juce::juce_audio_processors
        PUBLIC juce::juce_recommended_config_flags juce::juce_recommended_lto_flags
            juce::juce_recommended_warning_flags)
    add_dependencies(${target} ${target}_rust)
endfunction()
