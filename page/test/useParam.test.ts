import { act, renderHook } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import { createClient, type Client } from "../src/client";
import { PROTOCOL_VERSION } from "../src/generated/protocol";
import { createHooks } from "../src/hooks";
import { memoryTransport, type MemoryTransport } from "../src/transport";
import type { Params } from "./generated/params";

function setup(wrap: (client: Client) => Client = (client) => client) {
  const t = memoryTransport();
  const client = wrap(createClient(t.page));
  // The native side is listening already, so writes and calls are sent at once.
  t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [] });
  const { useParam } = createHooks<Params, Record<string, never>, Record<string, never>>(client);
  const writes = () => t.native.received.slice(1).map((m) => (m.type === "set" ? `set:${m.normalized}` : m.type));
  return { t, useParam, writes };
}

const state = (normalized: number, text = `t${normalized}`, param = "gain") => ({ param, normalized, text });

// A native side that answers as the protocol requires: every set with the
// snapped value (tenths here), every gesture end with whatever `store()`
// holds then.
function answer(t: MemoryTransport, store: () => number) {
  t.native.listen((m) => {
    if (m.type === "set") t.native.send({ type: "param", state: state(Math.round(m.normalized * 10) / 10) });
    if (m.type === "gestureEnd") t.native.send({ type: "param", state: state(store()) });
  });
}

describe("useParam", () => {
  test("seeds from the client cache on mount", () => {
    const { t, useParam } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.4)] });
    const { result } = renderHook(() => useParam("gain"));
    expect(result.current.value).toBe(0.4);
    expect(result.current.text).toBe("t0.4");
  });

  test("seeds from the snapshot when it arrives after mount", () => {
    const { t, useParam } = setup();
    const { result } = renderHook(() => useParam("gain"));
    expect(result.current.value).toBe(0);
    expect(result.current.text).toBe("");
    act(() => t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.4)] }));
    expect(result.current.value).toBe(0.4);
  });

  test("applies a native change while idle", () => {
    const { t, useParam } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => t.native.send({ type: "param", state: state(0.42) }));
    expect(result.current.value).toBe(0.42);
    expect(result.current.text).toBe("t0.42");
  });

  test("brackets one drag into begin, set..., end in order", () => {
    const { useParam, writes } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(0.25);
      result.current.onDrag(0.4);
      result.current.onDragEnd();
    });
    expect(writes()).toEqual(["gestureBegin", "set:0.25", "set:0.4", "gestureEnd"]);
  });

  test("a change during a drag never overwrites it; the drag end adopts the latest one", () => {
    const { t, useParam } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(0.3);
    });
    act(() => t.native.send({ type: "param", state: state(0.8) }));
    act(() => t.native.send({ type: "param", state: state(0.9) }));
    expect(result.current.value).toBe(0.3);
    act(() => result.current.onDragEnd());
    expect(result.current.value).toBe(0.9);
    expect(result.current.text).toBe("t0.9");
  });

  test("the answer to a drag's end wins over what arrived during the drag", () => {
    const { t, useParam } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.5)] });
    // A host value that landed mid-drag stands at the close, so the end's
    // answer is 0.5 while the drag's own answers were 0.3.
    answer(t, () => 0.5);
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(0.33);
    });
    expect(result.current.value).toBe(0.33);
    act(() => result.current.onDragEnd());
    expect(result.current.value).toBe(0.5);
    expect(result.current.text).toBe("t0.5");
  });

  test("a drag end with its answers still in flight keeps the dragged value until they land", () => {
    const { t, useParam } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.5)] });
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(0.33);
      result.current.onDragEnd();
    });
    expect(result.current.value).toBe(0.33); // not the pre-drag 0.5
    act(() => t.native.send({ type: "param", state: state(0.3) }));
    expect(result.current.value).toBe(0.3);
  });

  test("a one-shot set shows the native side's answer, even when the store did not move", () => {
    const { t, useParam } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.5)] });
    const { result } = renderHook(() => useParam("gain"));
    act(() => result.current.setValue(0.3));
    expect(result.current.value).toBe(0.3);
    act(() => t.native.send({ type: "param", state: state(0.5) })); // refused, or snapped back
    expect(result.current.value).toBe(0.5);
  });

  test("a non-finite position is neither shown nor sent", () => {
    const { t, useParam, writes } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.5)] });
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.setValue(Number.NaN);
      result.current.onDragStart();
      result.current.onDrag(Number.POSITIVE_INFINITY);
      result.current.onDragEnd();
    });
    expect(result.current.value).toBe(0.5);
    expect(writes()).toEqual(["gestureBegin", "gestureEnd"]);
  });

  test("a drag past either end shows and sends the end", () => {
    const { useParam, writes } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(1.4);
    });
    expect(result.current.value).toBe(1);
    act(() => {
      result.current.onDragEnd();
      result.current.setValue(-0.2);
    });
    expect(result.current.value).toBe(0);
    expect(writes()).toEqual(["gestureBegin", "set:1", "gestureEnd", "set:0"]);
  });

  test("re-sent identical states do not keep rendering", () => {
    const { t, useParam } = setup();
    let renders = 0;
    renderHook(() => {
      renders += 1;
      return useParam("gain");
    });
    act(() => t.native.send({ type: "param", state: state(0.4) }));
    const afterFirst = renders;
    for (let i = 0; i < 5; i += 1) act(() => t.native.send({ type: "param", state: state(0.4) }));
    // React may run the component once more before it bails out of an
    // unchanged state; a new state object per repeat would render five times.
    expect(renders - afterFirst).toBeLessThanOrEqual(1);
  });

  test("a second onDragStart or a stray onDragEnd sends nothing", () => {
    const { useParam, writes } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragEnd();
      result.current.onDragStart();
      result.current.onDragStart();
      result.current.onDragEnd();
    });
    expect(writes()).toEqual(["gestureBegin", "gestureEnd"]);
  });

  test("setValue outside a drag sends only a set", () => {
    const { useParam, writes } = setup();
    const { result } = renderHook(() => useParam("gain"));
    act(() => result.current.setValue(0.75));
    expect(result.current.value).toBe(0.75);
    expect(writes()).toEqual(["set:0.75"]);
  });

  test("unmount mid-drag sends gestureEnd and unbinds", () => {
    let binds = 0;
    let disposes = 0;
    const { t, useParam, writes } = setup((client) => ({
      ...client,
      bindParam(id, onState) {
        binds += 1;
        const bound = client.bindParam(id, onState);
        return {
          ...bound,
          dispose: () => {
            disposes += 1;
            bound.dispose();
          },
        };
      },
    }));
    const { result, unmount } = renderHook(() => useParam("gain"));
    act(() => {
      result.current.onDragStart();
      result.current.onDrag(0.6);
    });
    expect(disposes).toBe(0);
    unmount();
    expect(writes()).toEqual(["gestureBegin", "set:0.6", "gestureEnd"]);
    expect(binds).toBe(1);
    expect(disposes).toBe(1);
    // No listener left to throw on a later message.
    t.native.send({ type: "param", state: state(0.1) });
  });

  test("an id change mid-drag ends the old gesture, then binds the new id", () => {
    const { t, useParam, writes } = setup();
    t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION, params: [state(0.1), state(0.9, "p", "pan")] });
    const { result, rerender } = renderHook(({ id }: { id: "gain" | "pan" }) => useParam(id), {
      initialProps: { id: "gain" },
    });
    act(() => result.current.onDragStart());
    rerender({ id: "pan" });
    expect(t.native.received.at(-1)).toEqual({ type: "gestureEnd", param: "gain" });
    expect(result.current.value).toBe(0.9);
    expect(writes()).toEqual(["gestureBegin", "gestureEnd"]);
  });

  test("two controls on one parameter each send their own begin and end", () => {
    const { useParam, writes } = setup();
    const a = renderHook(() => useParam("gain"));
    const b = renderHook(() => useParam("gain"));
    act(() => {
      a.result.current.onDragStart();
      b.result.current.onDragStart();
      a.result.current.onDragEnd();
      b.result.current.onDragEnd();
    });
    expect(writes()).toEqual(["gestureBegin", "gestureBegin", "gestureEnd", "gestureEnd"]);
  });

  test("a protocol mismatch shows up as failure and writes are dropped", () => {
    const { t, useParam, writes } = setup();
    const { result } = renderHook(() => useParam("gain"));
    expect(result.current.failure).toBeUndefined();
    act(() => t.native.send({ type: "snapshot", protocol: PROTOCOL_VERSION + 1, params: [] }));
    expect(result.current.failure?.message).toMatch(/protocol mismatch/);
    act(() => result.current.setValue(0.5));
    expect(writes()).toEqual([]);
  });
});
