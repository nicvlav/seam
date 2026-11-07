import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Unmount every rendered tree so a hook's cleanup runs before the next test.
afterEach(() => {
  cleanup();
});
