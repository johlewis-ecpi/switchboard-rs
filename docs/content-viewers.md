# Modular content viewer

Switchboard owns selection, file access, the viewer window, loading/error states,
and **Open Externally**. Reader implementations stay outside the library UI.
Select a file and press Space, or choose **View in App** from Details or its context
menu. Space again, Escape (in the host UI), or Close dismisses the viewer. Plain
Space also closes from installed reader frames and HTML document content; typing
in text fields and activating buttons/media controls keeps normal behavior.
Repeated or modified Space key events do not toggle the viewer. Ordinary Open behavior is unchanged.

Opening a cloud-only file (including OneDrive files) allows macOS to download
its contents for this preview. Native documents are fetched on a worker before
Quick Look is attached; text and package readers fetch through their background
reads. The preview stays in its loading state while the provider responds. The
cloud provider must be running, signed in, and able to retrieve the file. Mounted
remote volumes remain excluded. Content search still skips cloud placeholders
and never enables downloading. Path checks on the UI thread remain metadata-only.

## Current adapters

| Content | Adapter | Status |
| --- | --- | --- |
| Word, PowerPoint, PDF, RTF | Native `QLPreviewView` | Embedded system preview, subject to installed macOS format support |
| HTML (`.html`, `.htm`, `.xhtml`) | `readers/html` | Visual page with inline CSS/JS in an isolated document frame; optional source view |
| Text/code | `readers/text` | Separate runtime package; wrapping and selectable source |
| Markdown | External package | Source fallback until installed |
| FreeMind `.mm` | External package | Source fallback until installed |
| GanttProject `.gan` | External package | Source fallback until installed |
| Draw.io `.drawio` | External package | Source fallback until installed |

Quick Look is a full preview view, distinct from the existing hover thumbnail
service. For formats handled by an enabled installed reader, hover details name
that reader and point to **View in App** instead of requesting an unrelated
Quick Look thumbnail. Reader API v1 has no thumbnail endpoint; a missing native
thumbnail does not mean the installed reader failed. Preview availability and pagination are controlled by macOS. The native
adapter does not convert documents, require Office, or write source files.

## Package discovery

Each reader is a directory containing `reader.json` and a self-contained HTML
build. Install directories in
`~/Library/Application Support/Switchboard/Readers/`, or set
`SWITCHBOARD_READERS` to another directory. Discovery runs on each document open;
reader changes do not require rebuilding Switchboard. One package may claim
multiple extensions. Duplicate extension claims produce an explicit error.

```json
{
  "apiVersion": 1,
  "id": "example.gantt",
  "name": "Gantt Viewer",
  "extensions": ["gan"],
  "entry": "index.html"
}
```

Extensions are case-insensitive and omit the dot. Entries and symlinks must stay
inside the package directory. Package HTML is limited to 16 MiB; document text to
4 MiB. Documents are UTF-8 or BOM-marked UTF-16. API v1 supplies text only; a
package claiming a binary extension receives a decode error (binary formats
need a reader that declares API v2, below). Native Office/PDF
adapters receive a file URL internally rather than passing binary data to JS.

Bundled fallback packages ship in `Contents/Resources/Readers`. Development uses
this repository's `readers/` directory. `SWITCHBOARD_BUNDLED_READERS` overrides the
bundled directory for development/testing. No web-reader source is compiled into the
HTML shell or Rust binary. The native Quick Look adapter is compiled Rust code. Only host bridge code is bundled into the shell.

## Reader API v1

Installed readers run in a sandboxed iframe with an opaque origin. They receive text and
display metadata, never filesystem paths, catalog access, or the application IPC
bridge. HTML/JS/CSS must be bundled into one HTML file; images/fonts can be data
URLs. Network, nested frames, form submissions, navigation of the parent window,
and filesystem access are unavailable. This is a display contract, not an editor
or general plugin API.

After registering its message listener, a reader announces readiness:

```js
window.addEventListener('message', event => {
  if (event.source !== parent || event.data?.type !== 'switchboard:open'
      || event.data.apiVersion !== 1) return;
  const {request, document} = event.data;
  // document: {name, extension, text, readOnly: true}
  // Call the reader's existing parser and renderer here.
  // Report a failure with the same request ID:
  // parent.postMessage({type:'switchboard:error', apiVersion:1,
  //   request, message:'Could not parse document'}, '*');
});
parent.postMessage({type: 'switchboard:ready', apiVersion: 1}, '*');
```

Register the listener before sending ready. The bundled text package is a
complete working reference. The host checks the message source and request ID,
drops late document responses, disposes frames on close, and times out readers
that do not announce readiness within 15 seconds. The wildcard target origin is
required because the reader's origin is opaque.

## Reader API v2: binary range reads

Readers for binary or very large files (ZIP archives, for example) declare
`"apiVersion": 2` in `reader.json`. For these readers the host sends **no file
contents**: `switchboard:open` carries only `document: {name, extension, size,
readOnly: true}`, and the reader asks for byte ranges as it needs them. The 4 MiB
text limit and UTF-8 requirement do not apply, so multi-gigabyte files open
immediately. Everything else (sandbox, CSP, 15 s readiness deadline, request
supersession, `switchboard:rendered`/`switchboard:error`) is unchanged, and every
v2 message carries `apiVersion: 2`. v1 and v2 readers coexist.

| Direction | Message |
| --- | --- |
| reader → host | `{type:'switchboard:read', apiVersion:2, request, id, offset, length}` |
| host → reader | `{type:'switchboard:range', apiVersion:2, request, id, data: ArrayBuffer}` |
| host → reader | `{type:'switchboard:range-error', apiVersion:2, request, id, message}` |

Host guarantees and limits:

- A read is accepted only from the current reader frame and request, and only
  before it is superseded. `id` is chosen by the reader; a duplicate in-flight `id`
  is dropped.
- `offset` and `length` must be safe integers with `offset + length <= size` and
  `length <= 8 MiB`; otherwise the reader gets `range-error: Invalid range`.
  Zero-length reads return an empty buffer. At most 8 reads may be outstanding;
  more are refused with `range-error`.
- The host re-resolves the document path for every read and re-checks bounds
  against the file's *current* length (a file that shrinks yields an error, never
  a short read). Readers never see paths.
- `data.byteLength` always equals `length`. Replies for closed or superseded
  viewers are dropped.
- Ranges cross the Rust/WebView boundary as base64, so prefer reads of about
  1 MiB; the reader in `switchboard-rs-viewer-zip` reads the central directory
  and individual entries this way.

Test with Sample works for v2 readers: any file extension the reader declares can be
chosen, with no size limit. Reference reader: `switchboard-rs-viewer-zip`
(`src/bridge.js` is a reusable v2 adapter). Host tests: `cargo test` (range reads,
v2 documents) and `node scripts/test-viewer-ranges.cjs` (page-side protocol).

## Reusing existing apps

The existing repositories were inspected, not copied or modified:

- `md-editor`: `MarkdownCore/MarkdownContent.swift` already shares parsing and
  rendering between preview/export. Expose that as a reusable library and a
  reader adapter; the current Swift executable is not an HTML reader package.
- `Gantt-Editor`: `src/domain/gan.ts` exports `parseGan`, and
  `src/domain/export.ts` exports `buildProjectSvg`. An independently built web
  reader can call these exports and return their SVG, without copying the parser
  or embedding the editor application.
- `flowmaker-rs`: Draw.io parsing lives in the Rust backend (`src/xml.rs`), while
  the UI depends on application state and server APIs. Extract a reusable
  parse/render library or a standalone reader build before registering it.
- No existing FreeMind `.mm` reader was identified. Flowmaker's `.drawio` support
  is a different format and cannot substitute for it.

Those format integrations are pending. The host deliberately contains no
replacement Markdown, GAN, FreeMind, or Draw.io parser. Reader extraction should
happen in the owning repositories, with their tests and release lifecycle; this
repository consumes only their published reader builds.

## Managing viewers in the app

Open **Switchboard → Settings…** (Command–Comma), then **Viewers**. Settings is
also available in the in-window File menu. The manager lists the system Quick
Look adapter, bundled text reader, and every installed package directory,
including packages with missing or invalid manifests. Each row shows its formats,
version when declared, source, installation path, enablement, validation errors,
format/ID conflicts, and last sample test.

- **Install…** opens a folder picker. Select a reader package containing
  `reader.json` and its self-contained HTML entry. Switchboard validates the
  package and copies its manifest and entry into the reader directory. The
  original package stays in place. Existing IDs are never overwritten; disable
  conflicting format handlers before installing another handler for that format.
- **Disable / Enable** controls normal file dispatch and persists across launches.
  System and bundled readers can be disabled; their packages cannot be removed.
  Broken packages can be disabled or removed so they do not block other readers.
- **Remove…** asks for confirmation, then deletes only the installed package and
  its saved test history. Referenced source documents are preserved.
- **Refresh** rescans the reader directory and validates current package contents.

### Test with Sample

Select a representative local file. Tests always use the chosen reader directly,
including disabled readers; a fallback cannot make a broken reader appear to
work. Test results are stored separately from the catalog in
`Readers/.viewer-state.json`, alongside enablement settings. They are not part of
library Undo or catalog backups.

The manager keeps these observations separate:

| Check | Meaning |
| --- | --- |
| Validation | Manifest API, extension declarations, entry path, file bounds and HTML encoding are valid. This does not execute the reader. |
| Startup passed | A web reader signaled `switchboard:ready` within 15 seconds. |
| Native view opened | The system preview view was created. Quick Look has no reader handshake, so this does not prove document rendering. |
| Reader reports success | The reader sent `switchboard:rendered` for the current request. Visual verification is still required. |
| Visually confirmed | You inspected the sample and clicked **Confirm Rendering**. |
| Rendering failed | The reader reported an error after startup, or you clicked **Report Problem**. |
| Not completed / Not verified | The test was closed early, the sample could not be loaded, or no rendering verdict was recorded. |

Results include the sample name/extension and time of the test. They apply to that
sample, not every document the reader claims to support. Changed reader HTML or
manifest content marks older results stale. Native results also become stale when
the OS version or native adapter version changes.

API v1 readers can optionally report rendering completion after processing the
`switchboard:open` message:

```js
parent.postMessage({
  type: 'switchboard:rendered', apiVersion: 1, request
}, '*');
```

Only the current reader frame and request are accepted. A reader cannot mark its
own output as visually confirmed. Readers without this optional message remain
compatible: startup can pass, and you can confirm rendering after inspection.
The bundled text reader implements both readiness and completion messages.

Manifests may optionally include a `version` string for display, for example
`"version": "1.0.0"`. Installed packages must declare at least one extension.

## Building a reader from the sample project

Start with [`examples/viewer-template`](../examples/viewer-template/README.md).
It is a standalone project with a working `.sbview` reader, separate reusable
bridge/parser/renderer modules, offline build tools, a sandboxed development
harness, test fixtures, JSON schemas, and a [complete API/format specification](../examples/viewer-template/SPEC.md).
Run `npm run check` inside that directory, then install its generated
`dist/example.sample-report` folder through Settings → Viewers. Creating/building
the template does not install it or change reader settings.

## Bundled HTML reader

HTML opens visually by default through `readers/html`, an independently loaded
bundled package. **Page** displays the page; **Source** shows selectable source
text. The package appears as **HTML Viewer** in Settings → Viewers and supports
disable/enable and direct sample tests. An installed HTML reader takes precedence;
its normal installed-package permissions apply. Disabling the bundled reader
falls back to text with an explanatory notice.

The bundled adapter alone receives a host-selected `html` renderer capability
that permits one additional `about:srcdoc` frame. Its document frame has a separate
opaque origin and a stricter CSP that blocks further frames, network fetches,
external scripts/styles/images, forms and document-directory access. Inline
styles and scripts can render/operate the page inside that frame. No manifest
field or package name can obtain this capability. Native navigation policy allows
local anchors and blocks external document navigation.

The input is still bounded decoded text (UTF-8 or BOM-marked UTF-16, up to 4 MiB).
HTML requiring neighboring assets or remote services needs **Open Externally**;
the reader receives no filesystem path or directory privileges. This bundled
adapter does not expand the generic API v1 package permissions.

## Viewer keyboard handling

The host injects a small keyboard adapter into its webview frames; packages do
not need to implement Space handling or receive application IPC credentials.
Reader key events are relayed only through current direct child frames, and the
host accepts dismissal only from the active reader frame. Editable controls,
buttons, links and media retain ordinary keys; key repeat and modifiers are
ignored for toggling. A scoped native event monitor handles Quick Look content
and is removed when its view closes. Test status persists through normal viewer
disposal.
