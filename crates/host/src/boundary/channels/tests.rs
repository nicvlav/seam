use super::*;

/// Room for the tests' blocks of up to four frames.
const FOUR: NonZeroUsize = NonZeroUsize::new(4).unwrap();

fn run(
    channels: &mut Channels,
    inputs: &[*mut f32],
    outputs: &[*mut f32],
    frames: usize,
) -> Vec<Vec<f32>> {
    // SAFETY: the tests' buffers are live, hold enough floats, and are
    // touched by nothing else.
    unsafe {
        channels.run(inputs, outputs, 0, frames, |block| {
            let seen = block.iter().map(|channel| channel.to_vec()).collect();
            for channel in block.iter_mut() {
                for sample in channel.iter_mut() {
                    *sample += 10.0;
                }
            }
            seen
        })
    }
}

#[test]
fn separate_buffers_copy_the_input_in() {
    let mut channels = Channels::new(2, 2, FOUR);
    let (mut left, mut right) = ([1.0f32; 4], [2.0f32; 4]);
    let (mut out_left, mut out_right) = ([0.0f32; 4], [0.0f32; 4]);
    let seen = run(
        &mut channels,
        &[left.as_mut_ptr(), right.as_mut_ptr()],
        &[out_left.as_mut_ptr(), out_right.as_mut_ptr()],
        3,
    );
    assert_eq!(seen, vec![vec![1.0; 3], vec![2.0; 3]]);
    assert_eq!(out_left, [11.0, 11.0, 11.0, 0.0]);
    assert_eq!(out_right, [12.0, 12.0, 12.0, 0.0]);
    assert_eq!(left, [1.0; 4], "the input is only read");
}

#[test]
fn in_place_buffers_are_processed_where_they_are() {
    let mut channels = Channels::new(2, 2, FOUR);
    let (mut left, mut right) = ([1.0f32; 4], [2.0f32; 4]);
    let pointers = [left.as_mut_ptr(), right.as_mut_ptr()];
    let seen = run(&mut channels, &pointers, &pointers, 4);
    assert_eq!(seen, vec![vec![1.0; 4], vec![2.0; 4]]);
    assert_eq!((left, right), ([11.0; 4], [12.0; 4]));
}

#[test]
fn crossed_buffers_read_every_input_before_writing() {
    // The host hands its left input as the right output, and the right
    // input as the left output.
    let mut channels = Channels::new(2, 2, FOUR);
    let (mut a, mut b) = ([1.0f32; 4], [2.0f32; 4]);
    // Each pointer taken once: a second `as_mut_ptr` would invalidate
    // the first under Rust's aliasing rules.
    let (pa, pb) = (a.as_mut_ptr(), b.as_mut_ptr());
    let seen = run(&mut channels, &[pa, pb], &[pb, pa], 4);
    assert_eq!(seen, vec![vec![1.0; 4], vec![2.0; 4]]);
    assert_eq!((b, a), ([11.0; 4], [12.0; 4]));
}

#[test]
fn a_crossed_input_survives_a_missing_one() {
    // No left input, and the right input is the buffer the host also
    // hands as the left output.
    let mut channels = Channels::new(2, 2, FOUR);
    let (mut a, mut b) = ([1.0f32; 4], [2.0f32; 4]);
    let (pa, pb) = (a.as_mut_ptr(), b.as_mut_ptr());
    let seen = run(&mut channels, &[std::ptr::null_mut(), pa], &[pa, pb], 4);
    assert_eq!(seen, vec![vec![0.0; 4], vec![1.0; 4]]);
}

#[test]
fn missing_shared_and_extra_buffers_fall_back_or_are_cleared() {
    // Mono in, stereo out, with the host giving one buffer for both
    // outputs and a third output the plugin does not have.
    let mut channels = Channels::new(1, 2, FOUR);
    let mut input = [3.0f32; 4];
    let (mut shared, mut extra) = ([9.0f32; 4], [9.0f32; 4]);
    let shared_pointer = shared.as_mut_ptr();
    let seen = run(
        &mut channels,
        &[input.as_mut_ptr()],
        &[shared_pointer, shared_pointer, extra.as_mut_ptr()],
        4,
    );
    assert_eq!(
        seen,
        vec![vec![3.0; 4], vec![0.0; 4]],
        "a channel with no input starts zeroed"
    );
    assert_eq!(shared, [13.0; 4], "the first channel got the buffer");
    assert_eq!(extra, [0.0; 4], "an output past the plugin's is cleared");
    let seen = run(&mut channels, &[std::ptr::null_mut()], &[], 2);
    assert_eq!(seen, vec![vec![0.0; 2], vec![0.0; 2]], "no buffers at all");
}

#[test]
fn outputs_that_overlap_without_being_one_buffer_get_a_channel_each() {
    // A host hands its two outputs one float apart in one buffer.
    let mut channels = Channels::new(0, 2, FOUR);
    let mut buffer = [0.0f32; 5];
    let base = buffer.as_mut_ptr();
    // SAFETY: one float into a buffer of five.
    let shifted = unsafe { base.add(1) };
    let seen = run(&mut channels, &[], &[base, shifted], 4);
    assert_eq!(seen, vec![vec![0.0; 4], vec![0.0; 4]]);
    assert_eq!(
        buffer,
        [10.0, 10.0, 10.0, 10.0, 0.0],
        "the first output is the left channel, and the right lives in scratch"
    );
}

#[test]
fn an_input_that_overlaps_another_channel_s_output_is_read_before_any_write() {
    // The right input starts one float into the left output.
    let mut channels = Channels::new(2, 2, FOUR);
    let mut left_in = [7.0f32; 4];
    let mut buffer = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0f32];
    let mut right_out = [0.0f32; 4];
    let base = buffer.as_mut_ptr();
    // SAFETY: one float into a buffer of six, so four floats on.
    let shifted = unsafe { base.add(1) };
    let seen = run(
        &mut channels,
        &[left_in.as_mut_ptr(), shifted],
        &[base, right_out.as_mut_ptr()],
        4,
    );
    assert_eq!(seen, vec![vec![7.0; 4], vec![2.0, 3.0, 4.0, 5.0]]);
    assert_eq!(buffer, [17.0, 17.0, 17.0, 17.0, 5.0, 6.0]);
    assert_eq!(right_out, [12.0, 13.0, 14.0, 15.0]);
}

/// How many floats the model test's pool holds, where a call starts in a
/// view of it, and how many frames it takes.
const FLOATS: usize = 18;
const OFFSET: usize = 2;
const FRAMES: usize = 3;
/// Where each view a model test's host hands out starts in the pool. The
/// first three are buffers of six floats. The fourth's call overlaps the
/// first's, and the fifth's overlaps the fourth's and touches the first's
/// and the second's at their ends.
const VIEWS: [usize; 5] = [0, 6, 12, 1, 3];
const POOL: usize = VIEWS.len();

/// What the pool holds at `float` before a call: a value no other float
/// holds.
fn initial(float: usize) -> f32 {
    (100 + float) as f32
}

/// The floats of the pool a call reads or writes in `view`.
fn floats(view: usize) -> std::ops::Range<usize> {
    let start = VIEWS[view] + OFFSET;
    start..start + FRAMES
}

/// Whether a call's floats in two views share one.
fn overlap(a: usize, b: usize) -> bool {
    floats(a).start < floats(b).end && floats(b).start < floats(a).end
}

#[test]
fn every_way_a_host_hands_its_buffers_meets_the_model() {
    // From a pool of five views, each slot of the host's lists is one of
    // them or null, with up to one slot more than the plugin has on each
    // side. So buffers come separate, in place, crossed, shared, missing,
    // extra and overlapping. Under miri, a sample of the cases, too many to
    // interpret.
    let stride = if cfg!(miri) { 863 } else { 1 };
    let mut case = 0usize;
    for (inputs, outputs) in [(0, 1), (1, 1), (1, 2), (2, 1), (2, 2)] {
        let mut channels = Channels::new(inputs, outputs, FOUR);
        for input_slots in [inputs, inputs + 1] {
            for output_slots in [outputs, outputs + 1] {
                let slots = input_slots + output_slots;
                for choice in 0..(POOL + 1).pow(slots as u32) {
                    case += 1;
                    if !case.is_multiple_of(stride) {
                        continue;
                    }
                    let pick = |slot: usize| {
                        Some(choice / (POOL + 1).pow(slot as u32) % (POOL + 1))
                            .filter(|&buffer| buffer < POOL)
                    };
                    let host_inputs: Vec<_> = (0..input_slots).map(pick).collect();
                    let host_outputs: Vec<_> = (input_slots..slots).map(pick).collect();
                    meets_the_model(
                        &mut channels,
                        (inputs, outputs),
                        &host_inputs,
                        &host_outputs,
                    );
                }
            }
        }
    }
    assert_eq!(case, 86_730, "every arrangement");
}

/// Runs one call over the pool, the host's lists naming a view of it or
/// none in each slot, and checks what the processor saw and what the pool
/// holds after against the model of what `run` promises.
fn meets_the_model(
    channels: &mut Channels,
    (inputs, outputs): (usize, usize),
    host_inputs: &[Option<usize>],
    host_outputs: &[Option<usize>],
) {
    let mut pool = [0.0f32; FLOATS];
    for (float, value) in pool.iter_mut().enumerate() {
        *value = initial(float);
    }

    // The model. Channel `c` lives in the host's output `c` when the plugin
    // has that output and its floats overlap no earlier channel's, and in
    // scratch otherwise. It starts as the host's input `c`, read before any
    // write, or zeroed. Every place then holds what the processor made of
    // it, and an output past the plugin's that overlaps no place is
    // cleared.
    let mut places: Vec<Option<usize>> = Vec::new();
    for channel in 0..inputs.max(outputs) {
        let taken = |view: usize| places.iter().flatten().any(|&place| overlap(place, view));
        let own = host_outputs
            .get(channel)
            .copied()
            .flatten()
            .filter(|&view| channel < outputs && !taken(view));
        places.push(own);
    }
    let expected_seen: Vec<Vec<f32>> = (0..places.len())
        .map(
            |channel| match host_inputs.get(channel).copied().flatten() {
                Some(view) if channel < inputs => floats(view).map(initial).collect(),
                _ => vec![0.0; FRAMES],
            },
        )
        .collect();
    let mut expected = pool;
    for (seen, place) in expected_seen.iter().zip(&places) {
        if let Some(view) = place {
            for (float, sample) in floats(*view).zip(seen) {
                expected[float] = sample + 10.0;
            }
        }
    }
    for &extra in host_outputs.iter().skip(outputs).flatten() {
        if !places.iter().flatten().any(|&place| overlap(place, extra)) {
            for float in floats(extra) {
                expected[float] = 0.0;
            }
        }
    }

    // The call, with each view's pointer taken from the pool's once.
    let base = pool.as_mut_ptr();
    let pointer = |view: &Option<usize>| match view {
        // SAFETY: each view starts inside the pool, at most 12 floats in,
        // and the pool holds 18.
        Some(view) => unsafe { base.add(VIEWS[*view]) },
        None => std::ptr::null_mut(),
    };
    let input_pointers: Vec<*mut f32> = host_inputs.iter().map(pointer).collect();
    let output_pointers: Vec<*mut f32> = host_outputs.iter().map(pointer).collect();
    // SAFETY: each non-null pointer starts a view with `OFFSET + FRAMES`
    // floats of the pool after it, and nothing else touches the pool until
    // the call returns. `FRAMES` is at most the scratch's four. Views that
    // overlap break `run`'s contract, which `run` must survive.
    let seen = unsafe {
        channels.run(&input_pointers, &output_pointers, OFFSET, FRAMES, |block| {
            let seen: Vec<Vec<f32>> = block.iter().map(|channel| channel.to_vec()).collect();
            for sample in block.iter_mut().flat_map(|channel| channel.iter_mut()) {
                *sample += 10.0;
            }
            seen
        })
    };
    let case =
        format!("{inputs} in, {outputs} out; inputs {host_inputs:?}, outputs {host_outputs:?}");
    assert_eq!(seen, expected_seen, "what the processor saw: {case}");
    assert_eq!(pool, expected, "the pool after: {case}");
}
