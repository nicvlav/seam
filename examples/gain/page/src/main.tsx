// The gain's page: a slider on the gain, which drags it as a user would, and
// a field for the plugin's label, through two commands.

import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { createClient, createHooks, webviewTransport } from "seam-page";

import type { Params } from "./generated/params";
import "./styles.css";

/** The commands `examples/gain/src/page.rs` adds. */
interface Commands {
  getLabel: { args: void; returns: string };
  setLabel: { args: string; returns: null };
}

const client = createClient(webviewTransport());
const { useParam, useCommand } = createHooks<Params, Record<string, never>, Commands>(client);

function GainSlider() {
  const gain = useParam("gain");
  return (
    <label className="row">
      <span className="name">Gain</span>
      <input
        type="range"
        min={0}
        max={1}
        step={0.001}
        value={gain.value}
        onPointerDown={gain.onDragStart}
        onChange={(event) => gain.onDrag(event.currentTarget.valueAsNumber)}
        onPointerUp={gain.onDragEnd}
        onPointerCancel={gain.onDragEnd}
      />
      <output>{gain.text}</output>
    </label>
  );
}

function LabelField() {
  const { run: fetchLabel } = useCommand("getLabel");
  const { run: storeLabel, error } = useCommand("setLabel");
  const [label, setLabel] = useState("");
  useEffect(() => {
    fetchLabel().then(setLabel, () => {});
  }, [fetchLabel]);
  const commit = () => {
    storeLabel(label).catch(() => {});
  };
  return (
    <label className="row">
      <span className="name">Label</span>
      <input
        type="text"
        value={label}
        onChange={(event) => setLabel(event.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
        }}
      />
      <output>{error ? "not saved" : ""}</output>
    </label>
  );
}

function Gain() {
  const failure = useParam("gain").failure;
  if (failure) return <p className="failure">{failure.message}</p>;
  return (
    <main>
      <h1>Seam Example Gain</h1>
      <GainSlider />
      <LabelField />
    </main>
  );
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Gain />
    </StrictMode>,
  );
}
