# Switchboard viewer starter

A standalone sample project for building reusable, offline Switchboard viewers.
It includes an installable `.sbview` report reader, a reusable API v1 adapter,
a local development harness, tests, JSON schemas, and a complete [specification](SPEC.md).
It has no npm dependencies. Node.js 20+ is used only for development; installed
viewers run in Switchboard's WKWebView without Node or a local server.

## Build and preview

Run these commands from this directory:

```sh
npm run check
npm run preview
```

Open the localhost address printed by `preview`. The default is
[http://127.0.0.1:4173](http://127.0.0.1:4173); use `PORT=0 npm run preview` to choose
an available port. Stop the preview with Control–C. No `npm install` is required.

The harness loads the reader inside an opaque-origin sandbox using Switchboard's
reader CSP. Select the normal, Unicode/markup, empty, or invalid sample, or choose
a local file. The startup status, rendering report, manual verdict and protocol
log are distinct. No document contents are written to the protocol log.
The harness is a development tool; the final acceptance check runs in Switchboard.

## Install the sample in Switchboard

1. Run `npm run build`.
2. Open Switchboard **Settings → Viewers → Install…**.
3. Select `dist/example.sample-report/` (the package folder, not `dist/`).
4. Click its **Test with Sample…** and select `samples/welcome.sbview`.
5. Inspect the report and click **Confirm Rendering**, or **Report Problem**.

The starter does not install itself or modify your existing reader registry.
Switchboard copies the built package; rebuilding this source directory does not
change the installed copy. To update it, remove the old installed reader and
install the newly built folder. Source samples remain in place.

## Build your own viewer

Copy this entire directory into your own repository, then:

1. Update `reader.json`: choose a unique ID, display name, version and supported
   extensions. Use your actual format, such as `gan`, `mm`, `drawio`, or `md`.
2. Replace `src/format.js` with an adapter to your existing parser and
   `src/renderer.js` with your existing read-only rendering component. Keep these
   reusable outside Switchboard. The supplied report parser is just an example.
3. Update `src/main.js` to connect those modules to the reusable `src/bridge.js`.
   Supported extensions are injected from the manifest at build time.
4. Add real samples and parser/renderer tests (including the preview sample options
   and server allowlist), then update the format section of
   `SPEC.md` and its document schema. Include malformed and unsupported inputs.
5. Run `npm run check`, use the browser harness, and test the built package in
   Switchboard before distribution.

The supplied builder concatenates four explicitly listed browser scripts. It is
not an npm/module bundler. If your parser/component uses imports or external
packages, replace the build with an appropriate bundler and inline **all** JS,
CSS, fonts and permitted images into one HTML entry. Network fetches, neighboring
JS/CSS assets, dynamic module imports, native app APIs and document-directory
access are unavailable in the installed reader.

API v1 receives decoded text only. It does **not** deliver ZIP-based Office
files, PDFs, arbitrary binary bytes, native Rust/Swift modules, or server access.
A Swift/Rust core needs a compatible browser/WASM build or a separately designed
host adapter; registering its existing executable as an HTML reader will not work.
The host CSP currently does not grant WebAssembly compilation permission, so a
WASM port also requires an explicit host capability design, not just this template.

## Project layout

| Path | Purpose |
| --- | --- |
| `SPEC.md` | Normative API v1 contract, sample format and acceptance criteria |
| `reader.json` | Source manifest; extensions are the build-time source of truth |
| `src/bridge.js` | Parent-message validation, readiness, cancellation, results |
| `src/format.js` | Pure sample-format parser; replace with your shared parser |
| `src/renderer.js` | DOM renderer; replace/reuse independently of the bridge |
| `src/main.js` | Small composition layer |
| `src/index.html`, `src/style.css` | Reader shell, accessible states and styling |
| `scripts/` | Build, package validation and loopback preview server |
| `preview/` | Host simulation with the same sandbox/CSP and protocol log |
| `schemas/` | Manifest, API-message and sample-format JSON schemas |
| `samples/` | Normal, Unicode/markup, empty, and invalid documents |
| `test/` | Unit and built-package integration tests |
| `dist/<reader-id>/` | Generated installable package; excluded from source control |

## Verification

`npm run check` builds and validates the package, then tests message boundaries,
read-only payloads, duplicate/stale requests, async cancellation, parser/resource
limits, error reporting, safe DOM output, filtering/wrapping, and reproducibility.
JSON schemas are supplied for editor/tooling integration; no schema-validation
library is installed. The executable parser and manifest checker enforce runtime
rules, including constraints that JSON Schema alone cannot express.

Before releasing, follow the browser/native acceptance matrix in `SPEC.md`.
A valid manifest is not a successful startup; a reader's rendering report is
not visual confirmation. Tests and the harness do not prove every real-world
file in the declared format will render correctly.

## License

MIT; see `LICENSE`. The build embeds this notice in the HTML so the notice remains
with the installed entry. Preserve licenses for any parser, renderer or assets
you add, and include their required notices in your distributable build.
