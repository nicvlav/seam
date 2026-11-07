# What the JUCE adapter workflow builds: the example EQ through
# adapters/juce, as the formats the adapter ships on macOS, for both Apple
# architectures.
cmake_minimum_required(VERSION 3.25)
project(seam_adapter_check VERSION 0.1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

# The JUCE commit bench/ builds against; the workflow reads it from there.
set(SEAM_CHECK_JUCE_COMMIT "" CACHE STRING "The JUCE commit to build against")
if(NOT SEAM_CHECK_JUCE_COMMIT)
    message(FATAL_ERROR "set SEAM_CHECK_JUCE_COMMIT")
endif()
FetchContent_Declare(juce
    GIT_REPOSITORY https://github.com/juce-framework/JUCE.git
    GIT_TAG ${SEAM_CHECK_JUCE_COMMIT})
FetchContent_MakeAvailable(juce)

include(${CMAKE_CURRENT_LIST_DIR}/../../adapters/juce/SeamJuce.cmake)
seam_add_juce_plugin(SeamAdapterCheck
    PACKAGE seam-example-eq
    SETTINGS "${CMAKE_CURRENT_LIST_DIR}/../../examples/eq/seam-juce.cmake"
    FORMATS VST3 AU)
