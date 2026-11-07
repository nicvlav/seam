import { act, renderHook } from "@testing-library/react";
import { useLayoutEffect } from "react";
import { describe, expect, test } from "vitest";

import { createClient, type Client } from "../src/client";
import { createHooks } from "../src/hooks";
import { memoryTransport } from "../src/transport";

interface Events {
  clipWarning: { channel: number; peakDb: number };
  otherEvent: { note: string };
}

function setup() {
  const t = memoryTransport();
  const client = createClient(t.page);
  // Counts subscriptions, to show a new handler closure does not resubscribe.
  const subscribes: string[] = [];
  const counting: Client = {
    ...client,
    onEvent(name, handler) {
      subscribes.push(name);
      return client.onEvent(name, handler);
    },
  };
  const { useEvent } = createHooks<Record<string, never>, Events, Record<string, never>>(counting);
  const emit = (name: string, payload: unknown) => act(() => t.native.send({ type: "event", name, payload }));
  return { useEvent, emit, subscribes, t };
}

describe("useEvent", () => {
  test("calls the handler once per event with its payload", () => {
    const { useEvent, emit } = setup();
    const received: Array<{ channel: number; peakDb: number }> = [];
    renderHook(() => useEvent("clipWarning", (p) => received.push(p)));
    emit("clipWarning", { channel: 0, peakDb: -0.2 });
    emit("clipWarning", { channel: 1, peakDb: -0.1 });
    expect(received).toEqual([
      { channel: 0, peakDb: -0.2 },
      { channel: 1, peakDb: -0.1 },
    ]);
  });

  test("only reacts to its own event name", () => {
    const { useEvent, emit } = setup();
    const received: unknown[] = [];
    renderHook(() => useEvent("clipWarning", (p) => received.push(p)));
    emit("otherEvent", { note: "x" });
    expect(received).toEqual([]);
  });

  test("calls the latest handler without resubscribing", () => {
    const { useEvent, emit, subscribes } = setup();
    const received: string[] = [];
    let tag = "first";
    const { rerender } = renderHook(() => useEvent("clipWarning", () => received.push(tag)));
    tag = "second";
    rerender();
    rerender();
    emit("clipWarning", { channel: 0, peakDb: 0 });
    expect(received).toEqual(["second"]);
    expect(subscribes).toEqual(["clipWarning"]);
  });

  test("unsubscribes on unmount", () => {
    const { useEvent, emit } = setup();
    const received: unknown[] = [];
    const { unmount } = renderHook(() => useEvent("clipWarning", (p) => received.push(p)));
    unmount();
    emit("clipWarning", { channel: 0, peakDb: 0 });
    expect(received).toEqual([]);
  });

  test("resubscribes when the name changes", () => {
    const { useEvent, emit } = setup();
    const received: unknown[] = [];
    const { rerender } = renderHook(
      ({ name }: { name: "clipWarning" | "otherEvent" }) => useEvent(name, (p) => received.push(p)),
      { initialProps: { name: "clipWarning" } },
    );
    rerender({ name: "otherEvent" });
    emit("clipWarning", { channel: 0, peakDb: 0 });
    emit("otherEvent", { note: "n" });
    expect(received).toEqual([{ note: "n" }]);
  });

  test("receives an event sent by a layout effect that runs after the hook's", () => {
    const { useEvent, t } = setup();
    const received: string[] = [];
    let tag = "first";
    const { rerender } = renderHook(() => {
      const mine = tag;
      useEvent("otherEvent", () => received.push(mine));
      // Layout effects run before passive ones; a subscription made in a
      // passive effect would miss this event.
      useLayoutEffect(() => {
        t.native.send({ type: "event", name: "otherEvent", payload: { note: "early" } });
      });
    });
    expect(received).toEqual(["first"]);
    tag = "second";
    rerender();
    expect(received).toEqual(["first", "second"]);
  });
});
