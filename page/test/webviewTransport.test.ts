import { describe, expect, test } from "vitest";

import { createClient } from "../src/client";
import { PROTOCOL_VERSION, type NativeMessage } from "../src/generated/protocol";
import { webviewTransport, type WebViewGlobals } from "../src/transport";

// A fake of what a plugin window's WebView gives the page: `ipc`, which keeps
// what the page posts, and the `__seam` the transport installs, which the
// native side delivers through.
function window(): { scope: WebViewGlobals; posted: string[]; deliver: (message: NativeMessage) => void } {
  const posted: string[] = [];
  const scope: WebViewGlobals = { ipc: { postMessage: (text) => posted.push(text) } };
  const deliver = (message: NativeMessage) => scope.__seam?.receive(JSON.stringify(message));
  return { scope, posted, deliver };
}

describe("webviewTransport", () => {
  test("sends each message as JSON text through ipc", () => {
    const { scope, posted } = window();
    const transport = webviewTransport(scope);
    transport.send({ type: "ready", protocol: PROTOCOL_VERSION });
    transport.send({ type: "set", param: "gain", normalized: 0.5 });
    expect(posted.map((text) => JSON.parse(text))).toEqual([
      { type: "ready", protocol: PROTOCOL_VERSION },
      { type: "set", param: "gain", normalized: 0.5 },
    ]);
  });

  test("delivers what the native side posts to every listener, in order", () => {
    const { scope, deliver } = window();
    const transport = webviewTransport(scope);
    const first: NativeMessage[] = [];
    const second: NativeMessage[] = [];
    transport.listen((message) => first.push(message));
    const stop = transport.listen((message) => second.push(message));
    const param = (normalized: number): NativeMessage => ({
      type: "param",
      state: { param: "gain", normalized, text: `${normalized}` },
    });
    deliver(param(0.25));
    stop();
    deliver(param(0.5));
    expect(first).toEqual([param(0.25), param(0.5)]);
    expect(second).toEqual([param(0.25)]);
  });

  test("drops native text that is not JSON, and goes on", () => {
    const { scope, deliver } = window();
    const transport = webviewTransport(scope);
    const got: NativeMessage[] = [];
    transport.listen((message) => got.push(message));
    scope.__seam?.receive("not json");
    deliver({ type: "event", name: "learned", payload: null });
    expect(got).toEqual([{ type: "event", name: "learned", payload: null }]);
  });

  test("drops sends outside a plugin window, where there is no ipc", () => {
    const scope: WebViewGlobals = {};
    const transport = webviewTransport(scope);
    expect(() => transport.send({ type: "ready", protocol: PROTOCOL_VERSION })).not.toThrow();
  });

  test("carries a client's handshake both ways", () => {
    const { scope, posted, deliver } = window();
    const client = createClient(webviewTransport(scope));
    expect(JSON.parse(posted[0] ?? "")).toEqual({ type: "ready", protocol: PROTOCOL_VERSION });
    deliver({
      type: "snapshot",
      protocol: PROTOCOL_VERSION,
      params: [{ param: "gain", normalized: 0.5, text: "-24 dB" }],
    });
    const gain = client.bindParam("gain", () => {});
    expect(gain.current()?.text).toBe("-24 dB");
    gain.set(0.75);
    expect(JSON.parse(posted.at(-1) ?? "")).toEqual({ type: "set", param: "gain", normalized: 0.75 });
    client.dispose();
  });
});
