// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { defineSwathMap } from "../src/swath-map.js";

// `just demo` (issue #35) opens this page before any granule exists, so the
// zero-config bounds fit has nothing to fit yet — it passes the view and the
// x-ray toggle as query params instead (?xray&center=lon,lat&zoom=n). Applied
// as plain attributes before define() so the element upgrades with them; a
// bare URL stays exactly the zero-config demo it was.
const map = document.querySelector("swath-map");
const params = new URLSearchParams(location.search);
if (params.has("xray")) {
  map?.setAttribute("xray", "");
}
for (const name of ["center", "zoom"]) {
  const value = params.get(name);
  if (value !== null) {
    map?.setAttribute(name, value);
  }
}

defineSwathMap();
