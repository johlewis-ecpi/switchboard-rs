# Switchboard for Rust

A local macOS desktop reference library, built from the behavior in
[`docs/functional-specification.md`](docs/functional-specification.md). Rust owns
the catalog, queries, rules, transfers, transactions, and background services.
A bundled HTML/CSS/JavaScript interface runs inside the system WKWebView through
wry/tao. There is no local web server, Node runtime, account, or telemetry in the app.

## Run

Requires macOS 14.4+, a current stable Rust toolchain, and Apple Command Line Tools.

```sh
cargo run --locked --bin switchboard
```

The default catalog is `~/Library/Application Support/Switchboard/library.json`.
Window preferences live alongside it in `preferences.json`. Open counts and last-opened
times are kept locally in `activity.json`; this starts with opens in this version. Backups use the
`Backups` subdirectory. Extracted document text is cached in
`~/Library/Caches/Switchboard/ContentSearch`.

File icons and successful document previews are cached across launches in
`~/Library/Caches/Switchboard/Thumbnails`. The app checks file identity, size,
and modification time before reuse, without reopening or parsing the document.
Minimal and Thumbnails views load document thumbnails as tiles appear; apps and
folders retain native icons. Edits, file replacements, or renderer updates regenerate the preview. Failed
previews are retried; an unwritable cache does not prevent rendering.

Library windows remember their position and size across launches using macOS
window-frame autosave. Additional windows have separate saved frames in creation
order. The quick launcher continues to position itself on the pointer's display.

Use a separate catalog when evaluating the port alongside the original app:

```sh
SWITCHBOARD_LIBRARY=/tmp/switchboard-demo/library.json cargo run --locked
```

An unreadable catalog opens in a protected state and cannot be overwritten by
normal mutations. Use **File → Import, Export and Backups → Restore** to recover.
Do not point two running processes at the same catalog; cross-process locking and
live external reconciliation are outside the source specification.

## Use

- Add files/apps or folders with the native picker, or drop local paths into the window. Folders remain single references. Add URL accepts HTTP/HTTPS and offers clipboard/title suggestions.
- Select an entry to edit its name, tags, collection, notes, and favorite status. Names and collections organize the catalog; source files stay in place. Command-click toggles selection; Shift-click selects a range. Double-click opens by default.
- Use the sidebar for collection, smart, kind, favorite, and any/all tag filters. Search metadata, document contents, or both. The status bar exposes content-search coverage, refresh, and cache clearing.
- **View** contains collection/tag/smart/rule managers, grouped bulk organization, per-display configuration, and Minimal. Minimal hides the in-window menu strip while keeping the macOS menu bar available. It wraps icons to the window width by default and reflows on resize. Pinch on the trackpad or use Command–Plus/Minus to resize icons (48–160); Command–0 resets to 80. The size is saved across launches. Configure icon size, or turn wrapping off to use one to eight fixed rows. Filtered drag reordering, arrow navigation, and reduced-motion-aware bloom follow the layout.
- **Recently opened** and **Frequently opened**, in the sidebar and View menu, sort successful opens by time or count. Revealing a file does not count as opening it.
- Opening a reference shows a Loading indicator on its tile and a status message. It clears when macOS accepts the open request or an error is reported; launching runs in the background so the window stays responsive.
- Local references show availability badges for available, online-only, missing, and inaccessible files. Details includes **Refresh status** and **Reveal in Finder**; missing references reveal the nearest existing containing folder. Checks use metadata and permissions, without reading document contents.
- **View in App** (Space) opens a separate preview window at 75% of the primary screen’s width and height, centered on both axes, including from Minimal mode. Close or Space returns focus to the library.
- Manage readers in **Settings → Viewers**: install packages, inspect validation and last sample-test results, enable/disable readers, or remove installed packages. **Test with Sample** checks startup and lets you visually confirm rendering; changed packages invalidate old results.
- Select a local file and press **Space**, or choose **View in App**, to open the modular content viewer. Word, PowerPoint, PDF and RTF use an embedded native Quick Look view; HTML renders through a bundled HTML reader with Page/Source controls, and text/code uses a separate reader package. Press Space again to close the viewer; text fields, buttons and media controls keep their normal Space behavior. Markdown, `.mm`, `.gan` and `.drawio` show source until a reader package is installed. See [reader packages and integration status](docs/content-viewers.md).
- Hover over an entry for 450 milliseconds to see details and file previews, including in Minimal mode. HTML is rendered visually; Word and PowerPoint use Quick Look document previews. On failure, Switchboard retries a temporary local snapshot, then tries saved Office preview images; Word also supports a native formatted excerpt and HTML supports browser rendering. Clicking, typing, scrolling, and leaving the window dismiss the preview.
- **Control–Option–Space** opens Quick Launcher. Arrow keys select, Return opens, Command-Return reveals in Switchboard, and Escape dismisses. Configure its registered global shortcut in the application menu. Closing windows keeps the process running; use **Quit Switchboard** to exit.
- Library Undo checks affected fields, preserves unrelated edits, and replays groups atomically. Text fields retain their own editing Undo. Additions themselves are preserved when undoing automatic rule effects.
- Rule execution is ordered and progressive. New rules start disabled. Preview manual rule changes before applying; stale previews are rejected.
- Import portable JSON, legacy internal JSON, or bookmark HTML through a preview. Export portable JSON, HTML, or CSV. Backups preserve the whole catalog and display configuration; restore never alters referenced content or reruns rules.
- Open Library Health explicitly to check local references or web links, review duplicate references, locate a replacement, or remove a checked reference. Web checks run only when requested.

## Build a macOS application

```sh
./scripts/build-app.sh
open dist/Switchboard.app
```

The script validates `APP_VERSION` (default `1.1.0`) and `APP_BUILD_NUMBER` (default
`1`), builds release mode, includes the license, writes the bundle metadata, and
ad-hoc signs and verifies the application. The Rust port uses bundle identifier
`local.switchboard.rust.app` so its application registration can coexist with the
original. Developer ID signing and notarization are separate distribution work.

The GitHub workflow tests and packages Apple silicon and Intel independently,
creates ZIPs and SHA-256 checksums, and publishes version tags after both builds
succeed. Its runner labels follow [GitHub's supported runner inventory](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

## Verify

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
node scripts/test-hover.cjs
node scripts/test-minimal-zoom.cjs
node scripts/test-opening.cjs
node scripts/test-viewer.cjs
node scripts/test-viewer-manager.cjs
node scripts/test-html-reader.cjs
node scripts/test-viewer-keys.cjs
# Requires native Quick Look access:
cargo test --locked --lib -- --include-ignored
SWITCHBOARD_LIBRARY=/tmp/switchboard-demo/library.json cargo run --locked -- --check
SWITCHBOARD_LIBRARY=/tmp/switchboard-demo/library.json cargo run --locked -- --smoke-test
```

Tests use synthetic catalogs and temporary directories. HTTP tests bind loopback
mock servers and do not contact the public Internet. The native smoke test opens
a temporary window, exercises all seven normal displays and nine management
panels, loads the viewer inventory, verifies a sandboxed reader handshake and rendering-completion message, reports the result, and exits. Run it with a separate test catalog.

## Implementation boundaries

This is a functioning Rust implementation, with several remaining differences
from exact Swift UI parity. Track these before treating it as a drop-in migration:

- The launcher uses a floating tao window, rather than an AppKit nonactivating `NSPanel`. It joins Spaces, places itself on the pointer's display, and restores the previous app on Escape/Copy, but exact full-screen and focus behavior needs device testing. macOS system-reserved shortcut inspection is delegated to registration failure, without a separate symbolic-shortcut preflight.
- Hover details stay inside the webview; they are not independent passive native panels. Document previews use a small embedded Swift helper calling the native QuickLookThumbnailing API, with bounded execution and explicit error reporting. The helper requests document thumbnails rather than file icons, retries local snapshots, and uses independent fallback renderers. Word fallback pagination can differ from Word; PowerPoint needs a saved preview image if both Quick Look attempts fail. Successful previews persist in the thumbnail cache; cancellation rejects late display but does not immediately stop every preview request.
- Search uses Unicode case/diacritic folding. macOS name sorting uses Foundation localized standard comparison, and relative dates use the local time zone and system calendar's first weekday. Exact search equivalence across every locale still needs migration testing.
- Library state is shared across windows; preferences are stored separately as typed JSON rather than UserDefaults. Inspector/sidebar widths differ slightly, and managers use explicit Save Changes rather than autosave on selection changes.
- The documented JSON forms, optional omissions, dates, enums, sets, action envelopes, and missing-reference behavior have synthetic compatibility tests. The original Swift executable and its fixture suite are absent from this repository, so cross-implementation round-trip parity, including every associated-action spelling, remains unverified.
- Native bookmark, PDFKit, cloud-placeholder, screen-reader, removable-volume, denied-permission, multi-monitor, and shortcut behavior still needs the manual acceptance checks in the specification. Automated tests cannot establish all those platform behaviors.

A standalone [viewer starter project](examples/viewer-template/README.md) includes
a working reader, build/preview tools, tests, schemas and a full specification for
building reusable reader packages.

## Source layout

| File | Responsibility |
| --- | --- |
| `src/model.rs` | Catalog types, codecs, integrity, tag normalization, address keys |
| `src/query.rs` | Scope/tag/date predicates, sorting, launcher ranking, Minimal ordering |
| `src/rules.rs` | Rule validation, ordered evaluation, overrides, missing targets |
| `src/store.rs` | Atomic commits, unreadable protection, field patches, backups, restore |
| `src/transfer.rs` | Portable/HTML/CSV codecs, merge remapping, stale-preview checks |
| `src/services.rs` | Native references/icons, bounded content/PDF/preview work, health, HTTP titles |
| `src/main.rs` | Shared store, windows, IPC commands, generations, background scheduling |
| `src/menu.rs`, `src/platform.rs` | macOS menus, shortcut/focus/screen integration |
| `src/ui.html` | Local interface and window interaction state |
| `tests/contracts.rs` | Synthetic contract and failure-path tests |
