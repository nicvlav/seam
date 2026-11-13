// How messages travel between the page and the native side.
//
// The page never knows what carries them: whatever does is a Transport, and
// nothing else in this package imports one. Two exist: the in-memory
// transport, for tests (a mock native side in a browser could use it too, but
// none is built), and the WebView transport, for a page in a plugin's window
// (`seam-webview`). The transport for a standalone app is planned.

import type { NativeMessage, PageMessage } from "./generated/protocol";

/** Carries messages between the page and the native side, in the order they
    were sent, both ways: the client's order and gesture guarantees rest on it. */
export interface Transport {
  /** Sends one message to the native side. Never throws; a message the
      native side cannot decode is its problem to report. */
  send(message: PageMessage): void;
  /** Calls `handler` for every message from the native side until the
      returned function is called. */
  listen(handler: (message: NativeMessage) => void): () => void;
}

/** Both ends of an in-memory transport: `page` is what the page's client
    uses, `native` stands in for the native side. Delivery is synchronous,
    in send order. */
export interface MemoryTransport {
  page: Transport;
  native: {
    /** Delivers a message to the page's listeners. */
    send(message: NativeMessage): void;
    /** Every message the page has sent, oldest first. */
    readonly received: PageMessage[];
    /** Calls `handler` for each message the page sends from now on. */
    listen(handler: (message: PageMessage) => void): () => void;
  };
}

/** An in-memory transport, for tests and for a mock native side. */
export function memoryTransport(): MemoryTransport {
  const pageListeners = new Set<(message: NativeMessage) => void>();
  const nativeListeners = new Set<(message: PageMessage) => void>();
  const received: PageMessage[] = [];
  return {
    page: {
      send(message) {
        received.push(message);
        for (const listener of [...nativeListeners]) listener(message);
      },
      listen(handler) {
        pageListeners.add(handler);
        return () => pageListeners.delete(handler);
      },
    },
    native: {
      send(message) {
        for (const listener of [...pageListeners]) listener(message);
      },
      received,
      listen(handler) {
        nativeListeners.add(handler);
        return () => nativeListeners.delete(handler);
      },
    },
  };
}

/** What a page in a plugin's window finds on `window`: `ipc`, which the
    WebView provides to post to the native side, and `__seam`, which
    `webviewTransport` installs for the native side to deliver through. */
export interface WebViewGlobals {
  ipc?: { postMessage(text: string): void };
  __seam?: { receive(text: string): void };
}

/** The transport of a page in a plugin's window, whose native side is
    `seam-webview`: each message goes out as JSON text through
    `window.ipc.postMessage`, and the native side delivers its own, in
    order, as JSON text to `window.__seam.receive`, which this installs.

    `scope` is where the two live, `globalThis` in a page; a test passes a
    fake. Outside a plugin's window, with no `ipc`, sends are dropped. A
    native message that is not JSON is dropped too: the client checks the
    shape of every message that is. */
export function webviewTransport(scope: WebViewGlobals = globalThis as WebViewGlobals): Transport {
  const listeners = new Set<(message: NativeMessage) => void>();
  scope.__seam = {
    receive(text) {
      let message: NativeMessage;
      try {
        message = JSON.parse(text) as NativeMessage;
      } catch {
        return;
      }
      for (const listener of [...listeners]) listener(message);
    },
  };
  return {
    send(message) {
      scope.ipc?.postMessage(JSON.stringify(message));
    },
    listen(handler) {
      listeners.add(handler);
      return () => listeners.delete(handler);
    },
  };
}
