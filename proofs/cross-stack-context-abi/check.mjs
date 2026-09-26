import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const scintillaPublic = read("./sources/scintilla-public.rs");
const scintillaInternal = read("./sources/scintilla-internal.rs");
const beamscalePublic = read("./sources/beamscale-public.gleam");
const beamscaleInternal = read("./sources/beamscale-internal.rs");
const beamscaleAuthority = read("./sources/beamscale-authority.tsp");

const requireText = (source, text, label) => {
  if (!source.includes(text)) throw new Error(label + " missing " + JSON.stringify(text));
};

for (const [source, label] of [[scintillaPublic, "Scintilla public SDK"], [scintillaInternal, "Scintilla internal core"]]) {
  requireText(source, "scintilla.run/context/v1", label);
}
if (scintillaInternal.includes('"scintilla.context/v1"')) {
  throw new Error("Scintilla internal core regressed to the retired context ABI identity");
}

for (const [text, label] of [
  ["bmscl.context/v1", "context ABI"],
  ["bmscl-module-contract-v1", "module contract"],
  ["bmscl.module-semantics/v2", "module semantics"],
]) {
  requireText(beamscaleAuthority, text, "BeamScale TypeSpec authority " + label);
  requireText(beamscalePublic, text, "BeamScale public SDK " + label);
  requireText(beamscaleInternal, text, "BeamScale internal core " + label);
}

console.log("cross-stack context/module ABI identity proof passed");
