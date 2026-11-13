// How messages travel between the page and the native side.
//
// The page never knows what carries them: whatever does is a Transport, and
// nothing in this package imports one. Only the in-memory transport exists
// today, for tests; a mock native side in a browser could use it too, but none
// is built. The transports for a plugin window and for a standalone app are
// planned.

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
