// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The authoring panel's stylesheet (#355): adopted by the document — the
 * panel renders into light DOM (#291) — and kept beside the other sheets so
 * the element file holds the element. Tokens only; the DRY gate holds. */
import { css } from "./styles";

export const PANEL_SHEET = css`
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) { display: block; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-toggle {
  display: block;
  width: 100%;
  margin: 0;
  padding: 0;
  border: 0;
  background: none;
  text-align: left;
  cursor: pointer;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6; font-weight: 700;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 90%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-toggle::before { content: "▸ "; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-toggle[aria-expanded="true"]::before { content: "▾ "; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-toggle[aria-expanded="true"] { margin-bottom: 8px; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-toggle:focus-visible {
  outline: 2px solid var(--swath-color-accent);
  outline-offset: 1px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-heading {
  margin: 0 0 8px;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6; font-weight: 700;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 90%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step {
  border: 1px solid color-mix(in srgb, var(--swath-color-fg-muted) 20%, transparent);
  border-radius: 6px;
  padding: 8px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step[data-permanent] {
  border-color: color-mix(in srgb, var(--swath-color-accent) 25%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-header {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 0;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6; font-weight: 700;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-header .swath-authoring-step-key {
  color: var(--swath-color-accent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-header button {
  margin-left: auto;
  border: none;
  background: none;
  cursor: pointer;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 80%, transparent);
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-header button:hover { color: var(--swath-color-danger); }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-summary {
  display: block;
  margin: 0 0 6px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5; font-style: italic;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 75%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-insert {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  margin: 0;
  padding: 0 8px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-insert button {
  padding: 2px 8px;
  border: 1px dashed color-mix(in srgb, var(--swath-color-fg-muted) 40%, transparent);
  border-radius: 999px;
  background: none;
  cursor: pointer;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 90%, transparent);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-insert button:hover {
  background: color-mix(in srgb, var(--swath-color-fg-muted) 12%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) label {
  display: block;
  margin: 0 0 6px;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 90%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) input,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) select {
  display: block;
  width: 100%;
  box-sizing: border-box;
  margin-top: 1px;
  padding: 3px 6px;
  border: 1px solid color-mix(in srgb, var(--swath-color-fg-muted) 30%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--swath-color-bg) 60%, transparent);
  color: inherit;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) input:focus-visible,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) select:focus-visible,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) button:focus-visible {
  outline: 2px solid var(--swath-color-accent);
  outline-offset: 1px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) input:disabled,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) select:disabled { opacity: 0.4; cursor: not-allowed; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-bands {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 10px;
  margin: 2px 0 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-when {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 10px;
  margin: 2px 0 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-when label {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: 1.6;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-when input {
  display: inline-block;
  width: auto;
  margin: 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-bands label {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: 1.6;
  cursor: pointer;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-bands input {
  display: inline-block;
  width: auto;
  margin: 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-field-help {
  display: block;
  margin: 0 0 2px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.4;
  font-weight: 400;
  letter-spacing: normal;
  text-transform: none;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 75%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-plain {
  display: block;
  margin: 0 0 6px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 75%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-narrative {
  margin: 0 0 10px;
  padding: 6px 8px;
  border-left: 2px solid color-mix(in srgb, var(--swath-color-accent) 45%, transparent);
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1.5; font-style: italic;
  color: color-mix(in srgb, var(--swath-color-fg) 90%, transparent);
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-narrative:empty { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-preview {
  margin: 0 0 10px;
  padding: 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-preview img {
  display: block;
  width: 128px;
  height: 128px;
  border: 1px solid color-mix(in srgb, var(--swath-color-fg-muted) 30%, transparent);
  border-radius: 6px;
  background:
    repeating-conic-gradient(color-mix(in srgb, var(--swath-color-fg-muted) 12%, transparent) 0% 25%, color-mix(in srgb, var(--swath-color-bg) 60%, transparent) 0% 50%)
    0 0 / 16px 16px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-preview img[hidden] { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-preview figcaption {
  margin: 2px 0 0;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 75%, transparent);
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-advanced-toggle {
  display: block;
  margin: 2px 0 6px;
  padding: 0;
  border: 0;
  background: none;
  cursor: pointer;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 70%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-advanced-toggle::before { content: "▸ "; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-advanced-toggle[aria-expanded="true"]::before {
  content: "▾ ";
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-field-note {
  display: block;
  margin: 1px 0 0;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: var(--swath-color-danger);
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-field-note:empty { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-error {
  margin: 0 0 6px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: var(--swath-color-danger);
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-step-error:empty { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-udf-drop {
  display: block;
  margin: 2px 0 0;
  padding: 8px;
  border: 1px dashed color-mix(in srgb, var(--swath-color-fg-muted) 40%, transparent);
  border-radius: 6px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 85%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-udf-drop[data-active] {
  border-color: var(--swath-color-accent);
  background: color-mix(in srgb, var(--swath-color-accent) 8%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-udf-drop input[type="file"] {
  margin-top: 4px;
  padding: 2px 0;
  border: 0;
  background: none;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-udf-module {
  display: block;
  margin: 2px 0 0;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.5;
  color: var(--swath-color-accent);
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-udf-module:empty { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) textarea {
  display: block;
  width: 100%;
  box-sizing: border-box;
  min-height: 3em;
  margin-top: 1px;
  padding: 3px 6px;
  border: 1px solid color-mix(in srgb, var(--swath-color-fg-muted) 30%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--swath-color-bg) 60%, transparent);
  color: inherit;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: 1.5;
  resize: vertical;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0 0 4px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row .swath-authoring-formula-line {
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.6; font-weight: 700;
  color: var(--swath-color-accent);
  white-space: nowrap;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row select,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row input {
  margin-top: 0;
  min-width: 0;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row .swath-authoring-formula-op {
  flex: 0 0 52px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row button {
  flex: none;
  border: none;
  background: none;
  cursor: pointer;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 80%, transparent);
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-row button:hover { color: var(--swath-color-danger); }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-add {
  display: block;
  margin: 0 0 6px;
  padding: 2px 8px;
  border: 1px dashed color-mix(in srgb, var(--swath-color-fg-muted) 40%, transparent);
  border-radius: 999px;
  background: none;
  cursor: pointer;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 90%, transparent);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-formula-add:hover {
  background: color-mix(in srgb, var(--swath-color-fg-muted) 12%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-submit {
  margin-top: 10px;
  width: 100%;
  padding: 7px 10px;
  border: 1px solid color-mix(in srgb, var(--swath-color-accent) 45%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--swath-color-accent) 10%, transparent);
  cursor: pointer;
  color: inherit;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1.5; font-weight: 600;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-submit:hover:not(:disabled) {
  background: color-mix(in srgb, var(--swath-color-accent) 20%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-submit:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-submit-reason {
  margin: 4px 0 0;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 80%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-submit-reason:empty { display: none; }
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-error {
  margin: 8px 0 0;
  padding: 6px 8px;
  border: 1px solid color-mix(in srgb, var(--swath-color-danger) 45%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--swath-color-danger) 10%, transparent);
  color: var(--swath-color-danger);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: 1.5;
  overflow-wrap: anywhere;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-empty,
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-hint {
  margin: 0 0 8px;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1.5;
  color: color-mix(in srgb, var(--swath-color-fg-muted) 80%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-template {
  display: block;
  width: 100%;
  margin: 0 0 8px;
  padding: 6px 10px;
  border: 1px dashed color-mix(in srgb, var(--swath-color-fg-muted) 40%, transparent);
  border-radius: 6px;
  background: none;
  cursor: pointer;
  color: inherit;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-template:hover {
  background: color-mix(in srgb, var(--swath-color-fg-muted) 12%, transparent);
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-services {
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-services li {
  display: flex;
  align-items: center;
  gap: 6px;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-service-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--swath-font-ui); font-size: var(--swath-text-sm); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-services button {
  margin-left: auto;
  padding: 2px 7px;
  border: 1px solid color-mix(in srgb, var(--swath-color-danger) 40%, transparent);
  border-radius: 4px;
  background: none;
  cursor: pointer;
  color: var(--swath-color-danger);
  font-family: var(--swath-font-ui); font-size: var(--swath-text-xs); line-height: 1.5;
}
:is(swath-authoring-panel, .swath-authoring-inspector, .swath-authoring-strip) .swath-authoring-services button:hover {
  background: color-mix(in srgb, var(--swath-color-danger) 12%, transparent);
}
/* Shell regions (#291): the strip over the map, the inspector column. */
.swath-authoring-strip { display: grid; gap: var(--swath-space-2); }
.swath-authoring-chips {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--swath-space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}
.swath-authoring-chip {
  min-block-size: var(--swath-space-7);
  padding: 0 var(--swath-space-3);
  border: var(--swath-border-hairline);
  border-radius: var(--swath-radius-pill);
  background: var(--swath-color-bg-raised);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-xs);
  cursor: pointer;
}
.swath-authoring-chip[aria-pressed="true"] {
  border-color: var(--swath-color-accent-border);
  background: var(--swath-color-accent-bg);
  color: var(--swath-color-accent);
}
.swath-authoring-chip[data-invalid="true"] { border-color: var(--swath-color-danger); }
.swath-authoring-chip-gap { display: inline-flex; }
.swath-authoring-canvas {
  block-size: calc(var(--swath-space-8) * 5);
  border: var(--swath-border-hairline);
  border-radius: var(--swath-radius-md);
}
.swath-authoring-canvas swath-canvas-node { max-inline-size: calc(var(--swath-space-8) * 7); }
.swath-authoring-canvas .swath-authoring-chip { white-space: normal; text-align: start; }
.swath-authoring-canvas swath-canvas-node[data-orphan="true"] { opacity: 0.5; }
.swath-authoring-chip[data-orphan="true"] { text-decoration: line-through; }
.swath-authoring-inserts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--swath-space-2);
  align-items: center;
}
.swath-authoring-inserts .swath-authoring-insert {
  display: inline-flex;
  align-items: center;
  gap: var(--swath-space-1);
}
.swath-authoring-insert-label {
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-xs);
  color: var(--swath-color-fg-muted);
}
.swath-authoring-inspector { display: grid; gap: var(--swath-space-2); }
.swath-authoring-inspector .swath-authoring-step { margin: 0; }
:is(swath-authoring-panel) .swath-authoring-steps[hidden] { display: none; }
`;
