# seam-page

The page side of a Seam plugin UI: a client for the native protocol and React hooks (`useParam`, `useEvent`, `useCommand`) typed against the page's own parameter, event and command maps. Rust owns the parameters; the page only mirrors and edits them.

## The Transport seam

The package talks to the native side through `Transport` (`send` a `PageMessage`, `listen` for `NativeMessage`s), which keeps the order of messages both ways, and imports nothing else. Only `memoryTransport()` exists today, for tests and a browser mock backend. A transport for a plugin window and one for a standalone app are planned, along with the native side of the protocol, which is not built yet. The message types in `src/generated/protocol.ts` are generated from Rust; do not edit them.

## Recipe

Implement a `Transport` for your host, create the client from it, then call `createHooks<Params, Events, Commands>(client)` once at module scope and export the three hooks it returns. `Params` is the `Params` type of the plugin's generated parameter map (any interface keyed by id works). `Events` is an interface keyed by id, whose values are payload types. `Commands` maps each name to `{ args, returns }`: `args: void` for none, and `returns: null` for a command with no result, which resolves `null` because the native side always sends a value. Components call `useParam("gain")`, `useEvent("clipWarning", handler)` and `useCommand("getOrder")`; an id that is not in its map is a compile error.

The client sends `ready` after it starts listening, and again every second until a snapshot or a `reject` arrives. It holds its writes and calls until the snapshot arrives and then sends them in order, bounded as `src/held.ts` states (`HELD_PER_PARAM`, `HELD_INVOKES`); a reject drops them and rejects the calls. It limits every position to 0 to 1, which is all the native side accepts. It fails visibly (`client.failure()`, `useParam().failure`) when the native side rejects its protocol version. The native side answers a `ready` of another version with a `reject` carrying its own version, and every other `ready` with a snapshot. It answers every `set` and `gestureEnd` with the parameter's state, so a control ends on the store.

## Not built yet

Each needs a protocol message the Rust side does not have today.

- `useValue` (plain page-bound values): a value snapshot entry and a `param`-like `value` message, plus a `setValue` page message.
- `useObservable` (revisioned containers): an observable snapshot, an update message carrying `{ rev, op, value }`, and a page `setObservable` message for writable ones.
- `useMeter` (streamed meter batches): a `meter` native message with a sequence number and a batch of values, and a subscribe/unsubscribe page message.
- `useMenu` and `SeamContextMenu` (native context menus): a page `menuGather` or `menuShow` message naming the target, and a native reply listing the entries and a `menuInvoke` page message for the pick.
