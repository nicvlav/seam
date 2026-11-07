# The comparison

Three VST3 builds of one EQ, loaded by one console host, so Seam's cost and behaviour
can be measured against plain JUCE:

- **A.** The EQ written straight in JUCE, with no Seam: `bench/juce-eq`.
- **B.** The Rust EQ (`examples/eq`) through Seam's own VST3 exporter.
- **C.** The Rust EQ through Seam's C interface and the generic JUCE adapter.

And one reference build, for timing only:

- **A-native.** A built with `BENCH_JUCE_NATIVE_RANGES`: its parameters use JUCE's own
  `NormalisableRange` with a skew for the centre, as a JUCE developer would declare
  them, instead of the port of Seam's ranges that A needs to store what the Rust EQ
  stores (rule 1 below). JUCE converts a host's write 8 times on its way into an
  `AudioProcessorValueTreeState` parameter (the wrapper's read and write, then the
  state's own listener), where Seam's exporter converts it twice and the adapter four
  times. Each conversion through the port does the same arithmetic as Seam's ranges,
  which MSVC runs at about 1.4 times Seam's own cost here, so A pays Seam's range cost
  8 times over, where JUCE's own range is cheap. A-native is plain JUCE when parameters
  move.

And three more for timing only, A-port, A-port-clang and C-clang, which take the gap
between A-native and C apart one cause at a time ("The ladder" below).

The first run is headless, on the bare EQ: parameters and saved state only, no editor.
A second run opens each build's UI: Seam's page in B and C, JUCE's generic editor with
native sliders in A. Those UIs differ, so that run sets a WebView page against a native
UI, not like against like.

## What has to match first

A, B and C must produce the same output within -120 dBFS RMS and -90 dBFS peak before
any timing counts. At low frequencies an f32 biquad one ulp off in one coefficient
already misses that bar, so the practical target is bit-identical output, and the
first check is that A stores and computes exactly what the Rust EQ does.

`scripts/parity.py` checks it, case by case, against
`examples/eq/examples/parity/`:

| Check | Cases | What it compares |
| --- | --- | --- |
| ranges | 25 | Every parameter's key, kind, range, skew centre and default |
| snap | 204,128 | Every lattice point of every range, and a quarter, half and three quarters past it, through the range's snap |
| restore | 204,128 | The same values set as JUCE's own code or a state restore sets them: to 0..1 and back |
| normalized | 204,825 | A host's 0..1 through `setValue`, on even steps and seeded ones |
| coefficients | 186,936 | Every lattice point of each band parameter, and seeded combinations, at 44.1, 48 and 96 kHz |
| audio | 569,729 | Both processors, driven directly with no host, playing the same seeded noise through 600 blocks of 1 to 1,024 frames at each of 44.1, 48 and 96 kHz, with random host writes to every parameter between blocks and an occasional reset |

Every case matched, bit for bit; the audio's difference is -inf dBFS. A planted
difference (an off band skipped instead of run bypassed) shows as 124,000 to 163,000
mismatched frames per rate, so the check can fail.

A-native stores values a little apart from Seam's, so its output misses the bar (the
report gives by how much). It is timed beside the others and kept out of
every match check. `compare.py` reports how far it is from A, and checks, with JUCE's
range worked in f32, that none of its writes would leave its stored value unchanged,
so it skips no work A does.

## The rules A follows

Each is needed for that match:

1. **Values.** Every float parameter snaps and maps 0..1 through Seam's own range
   (`juce-eq/SeamRange.h`, a port of `crates/params/src/decl.rs`): the lattice in whole
   numbers of its finest decimal place, nearest with ties to the higher, the end
   always reachable, the skew in double.
2. **Coefficients.** From JUCE's `ArrayCoefficients<float>` makers, with float
   frequency, Q and gain factor (`Decibels::decibelsToGain`) and a double sample rate,
   each frequency clamped to the Nyquist frequency first, as the Rust EQ does.
3. **When.** At the start of `processBlock`, for each band whose values moved; never in
   a parameter listener.
4. **Filters.** One `IIR::Filter<float>` per channel per band, one `process` call per
   block, in band order. A band that is off still runs, bypassed through its process
   context, as in a `ProcessorChain`, so it comes back on warm.
5. **Output.** A plain multiply after the bands, with no ramp.
6. **Build.** x64, `/fp:precise` (which in Visual Studio 2022 contracts nothing into
   fused multiply-adds), and the dynamic C runtime, the one Rust's `sinf`, `cosf`,
   `tanf` and `powf` come from. JUCE's recommended LTO flags,
   as JUCE's own plugin example links them, since Rust's release profile builds B and C's
   Rust with fat LTO; the adapter links them on C too.
7. **Denormals.** `processBlock` starts with `ScopedNoDenormals`, as JUCE's project
   template does, because B's exporter flushes denormals to zero around every `process`
   call. Both read the host's parameter changes outside the flush. The parity check
   drives the Rust processor with no exporter, so with no flush; it still matches,
   because its noise and filters produce no subnormal, and both snap their filter state at
   the end of each block.
8. **Platform.** x86-64 only: elsewhere JUCE does not snap filter state, and compilers
   contract or rewrite the arithmetic.

## Build and run

Visual Studio 2022 and CMake 3.25 or later. `SEAM_BENCH_JUCE_GIT` may name a local
checkout that has JUCE 9.0.0 (`f8f8864`) instead of the network:

```sh
cmake -S bench -B bench/build -G "Visual Studio 17 2022" -A x64
cmake --build bench/build --config Release --target BenchParity BenchParityPort BenchHosted BenchJuceEq_VST3 BenchJuceEqNative_VST3 BenchJuceEqPort_VST3 BenchSeamJuceEq_VST3
python bench/scripts/parity.py --tool bench/build/BenchParity_artefacts/Release/BenchParity.exe
python bench/scripts/parity.py --tool bench/build/BenchParityPort_artefacts/Release/BenchParityPort.exe
python bench/scripts/hosted.py --tool bench/build/BenchHosted_artefacts/Release/BenchHosted.exe --bundle <a .vst3> --ids juce
```

The ladder's clang-cl builds come from a second folder, configured in a shell with
Visual Studio's x64 environment (`vcvars64.bat`) and LLVM's `bin` folder on the path;
it can take the first folder's JUCE instead of cloning it again:

```sh
cmake -S bench -B bench/build-clang -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DCMAKE_LINKER=lld-link \
  -DCMAKE_AR=llvm-lib -DCMAKE_RC_COMPILER=llvm-rc -DCMAKE_MT=llvm-mt \
  -DCMAKE_SHARED_LINKER_FLAGS=/opt:lldlto=3 -DCMAKE_MODULE_LINKER_FLAGS=/opt:lldlto=3 \
  -DCMAKE_EXE_LINKER_FLAGS=/opt:lldlto=3 \
  -DFETCHCONTENT_SOURCE_DIR_JUCE=<this checkout>/bench/build/_deps/juce-src
cmake --build bench/build-clang --target BenchParityPort BenchJuceEqPort_VST3 BenchSeamJuceEq_VST3
python bench/scripts/parity.py --tool bench/build-clang/BenchParityPort_artefacts/Release/BenchParityPort.exe
```

A lands in `bench/build/BenchJuceEq_artefacts/Release/VST3/`, A-native, as
`Bench JUCE EQ native.vst3`, in `bench/build/BenchJuceEqNative_artefacts/Release/VST3/`, and C, as
`Seam Example EQ JUCE.vst3`, in `bench/build/BenchSeamJuceEq_artefacts/Release/VST3/`.
A-port lands, as `Bench JUCE EQ port.vst3`, in `BenchJuceEqPort_artefacts/Release/VST3/`;
the clang-cl folder holds A-port-clang, as `Bench JUCE EQ port clang.vst3`, and C-clang
under the same paths.
C's target runs cargo itself (`adapters/juce/SeamJuce.cmake`), so it needs the Rust
toolchain too; its Debug configuration builds and plays the same as its Release one.

B's module is the EQ crate's `cdylib` (`cargo build -p seam-example-eq --release`),
which a bundle step puts in `Seam Example EQ.vst3/Contents/x86_64-win/`. Until
`cargo xtask bundle` exists, `scripts/compare.py` builds it and makes that bundle itself.

`scripts/hosted.py` loads one build through JUCE's VST3 hosting (`hosted/hosted.cpp`,
the `BenchHosted` target), names its parameters by the ids that build gives them
(`--ids seam` for B, `juce` for A and C), and checks it as a host sees it. A, B and C each
play the parity script's audio bit for bit against the Rust EQ driven directly, at all
three rates, and reload their own saved state.

## The comparison run

```sh
python bench/scripts/compare.py --tool bench/build/BenchHosted_artefacts/Release/BenchHosted.exe
```

It runs `hosted.py`'s check on A, B, C and C-clang first, and times nothing unless all
four match. Then the console host (`bench-hosted measure`) plays each of the seven
builds at 32 to 1,024 frames per block at 48 kHz, in three scenarios: no parameter
moving, the mid band's frequency moving every block, and all 18 continuous parameters
moving every block, and at two paces (console host rule 8; `--paces spin` runs one).
Each run is 0.25 s of untimed warm-up, then at least 2 s and 1,000 blocks timed, three
rounds by default: about an hour and three quarters in all, or under an hour at one
pace. A, B, C and C-clang must match each other bit for bit in every run of every
round, and so must A-native, A-port and A-port-clang.

It writes `report.md` and `report.json` into `bench/build/compare/`: the output match,
time per block (median and 99th percentile), the whole process's CPU and the main and
other threads' shares, the first block's time, load time, the memory the plugin adds,
binary size and lines of plugin code. Reports kept from past runs are in `reports/`,
each with its `report.json`.

## The rules the console host follows

Each keeps A, B and C driven alike. C is built by JUCE's wrapper, so where the wrapper
decides it behaves as A does:

1. **Parameters by id.** A and C list JUCE's hidden bypass parameter, 26 parameters to
   B's 25, and name each by JUCE's hash of its key, which JUCE offers no way to set,
   where B uses Seam's own hash, so the host addresses each by its key's id in that
   build, never its index.
2. **Automation through `process` only.** A and C apply a controller write on the
   message thread whenever the transport is stopped; B ignores one while the host is
   processing. JUCE's hosting sends every `setValue` both ways: in the next `process`
   call's parameter queue, and to the controller from a 60 Hz timer on the message
   thread. The console host reports the transport playing and plays a block before its
   first write, so A and C take the queue only, as B does.
3. **No resent values.** B's EQ retunes a band at any write, since it compares change
   counters; A only when a value changes, and C's writes come through JUCE's wrapper,
   which drops a value equal to the current one. A host that resends an unchanged value
   would charge B for retunes A and C skip. `compare.py` draws every write from the
   parity example's table of 0..1 steps and the values the EQ stores for them, and sends
   only one that moves the stored value.
4. **The process context.** A and C ask for a full process context and B for none. The
   host fills it for every build, so none pays for what another skips.
5. **Block size.** The host tells every build the largest block it will send. B and C
   play a longer one in pieces, and JUCE treats the size as an estimate.
6. **Values a `float` holds.** JUCE's wrapper hands A and C a host's 0..1 as `float`,
   where B gets the host's `double`. The host sends only values a `float` holds
   exactly, as the parity script does, so all three store the same.
7. **The tick.** C ticks the plugin 60 times a second on a `juce::Timer`, which
   runs on the timer thread of the plugin's own copy of JUCE and posts to the message
   thread; B ticks on its own window timer, every 15 ms, on the message thread. A has no
   tick, but its `AudioProcessorValueTreeState` copies moved values into its state tree on
   a timer, at 50 Hz while values change and backing off to 2 Hz when none do. A CPU
   figure taken across the whole process counts all of them; time per block does not.
   The console host runs the message loop on its main thread while it plays, so every
   timer runs, as in a host.
8. **Playback.** Each run is a fresh process that creates the plugin from a description
   that names its bundle, so the module loads cold, as when a host opens a project. A
   scan names the bundle only when it holds a `moduleinfo.json` (A's and C's do, B's has
   none until the bundle step writes one), and JUCE's hosting searches a bundle on every
   create, so `compare.py` names the bundle for every build. An audio thread at MMCSS
   "Pro Audio" priority starts each block when its frames would be due, so blocks are
   paced as in playback rather than run back to back. Between blocks it either spins,
   so its core stays awake and a block's time is the code's alone, or sleeps on a
   high-resolution timer, as in a light session, where the core may drop into a sleep
   state and wake with cold caches. How deep it sleeps varies with the machine's state:
   one run of the same builds saw 32-frame blocks take 3.6 µs in two rounds and 1.1 µs
   in the third, with every build moving alike, so the spinning pace leads and the
   sleeping one is reported beside it. Spinning is also the warm-cache case, the kindest
   to a build with a larger footprint; a host running many plugins per callback keeps
   the core awake but evicts each plugin's caches between calls, so it sits between the
   two. Only `processBlock` is timed, between fenced reads of the processor's time stamp
   counter, since Windows' clock steps in 100 ns, 10 to 15% of a 32-frame block. It
   includes JUCE's hosting of the call, the same for every build.
9. **Turns.** The builds take turns in an order rotated by an even share each round,
   so each build's places spread over the order, at every block size and scenario, on one machine in one session, and each figure is a median over
   the rounds. Every run times at least 1,000 blocks, so its 99th percentile is not its
   maximum.
10. **Buffers.** JUCE's hosting hands every build one buffer as both input and output.
    Seam's exporter processes it in place; JUCE's wrapper copies the input into its own
    buffer and the result back out, in A and C alike. A host that passes separate buffers
    would make B copy too.

## The ladder

A-native and C differ in three ways at once: the filter code (JUCE's `dsp::IIR` against
the Rust EQ's own), the compiler (MSVC against LLVM) and the framework. Three builds,
for timing only, take those apart:

- **A-port.** A-native with the Rust EQ's filters (`examples/eq/src/dsp.rs`) written
  line for line in C++ (`juce-eq/EqPort.h`) in place of JUCE's `dsp::IIR`: the same
  coefficient formulas, computed once per band; one section over both channels, one
  call per band per block; the bypass a flag inside the loop. Its parameters stay
  JUCE's own, as in A-native.
- **A-port-clang.** A-port built by clang-cl, which is LLVM, as Rust's compiler is.
- **C-clang.** C with its C++, JUCE's wrapper and the adapter, built by clang-cl. Its
  Rust is the same.

Each step from plain JUCE to Seam's own build changes one thing:

| Step | Ratio | What changes |
| --- | --- | --- |
| port | A-port / A-native | JUCE's filter code against the port, both MSVC |
| compiler | A-port-clang / A-port | clang-cl against MSVC, on the same C++ |
| framework | C-clang / A-port-clang | Seam's parameters, the adapter and the C interface, with the filters in Rust, against JUCE's `AudioProcessorValueTreeState` with the same filters in C++: all LLVM, inside the same clang-built wrapper |
| wrapper compiler | C / C-clang | C's wrapper and adapter built by MSVC against clang-cl |
| exporter | B / C | Seam's VST3 exporter against JUCE's wrapper and the adapter |

The rules those builds follow:

1. **The same work.** `BenchParityPort` is the parity tool on the port with Seam's
   ranges; `parity.py` checks it case for case against the Rust EQ, built by each
   compiler, and every case matches bit for bit. In every timed run A-native, A-port
   and A-port-clang must match each other bit for bit, and C-clang must match A.
2. **The same flags.** JUCE's recommended config and LTO flags under each compiler,
   `/fp:precise`, the dynamic C runtime, and no CPU-specific flags. clang-cl takes
   JUCE's `/Ox` as `-O3`, Rust's level, and the clang-cl folder links with
   `/opt:lldlto=3`, so its link-time optimisation runs at that level too, as Rust's fat
   LTO does; lld-link's own default is 2.
3. **Close compilers, not the same one.** Here rustc 1.99's LLVM is 23.1 and clang-cl's
   22.1, so the compiler step narrows the gap between the languages' toolchains without
   closing it.
4. **What the framework step still holds.** C's call crosses the C interface, which no
   link-time optimisation inlines across, and the Rust and C++ filters are two sources
   compiled apart, so that step counts those with the parameter path.
