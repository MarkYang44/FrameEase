# Desktop polish implementation plan

**Goal:** Use the supplied artwork for desktop icons, fix the reported layout defects, support complete English, and rewrite the public README.

**Approach:** Keep the existing Tauri/Rust engine and React interface. Move notifications outside the shell grid; constrain the inspector and its controls to the available width while keeping vertical scrolling and portaled menus. Complete the existing translation mechanism, including presets, notices, dialogs, accessibility labels, and time units. User filenames and paths remain verbatim.

- [x] Reproduce the layout defects in a real browser; add browser regressions for navigation, resizing, and long inspector content.
- [x] Add translation coverage and placeholder checks, then complete both dictionaries and make all views react to locale changes, including memoized queue rows.
- [x] Generate the macOS ICNS, Windows ICO and supporting PNGs from the supplied square artwork using Tauri's icon generator. Keep a source copy for repeatable generation.
- [x] Rewrite README in English with a Chinese companion, following the reference repository's overview, screenshots, features, setup, compatibility, development and license structure. Preserve verified platform limits.
- [x] Run frontend checks, browser regressions, Rust checks appropriate to backend text changes, and a macOS bundle build; inspect the resulting icons and review the diff.

**Evidence:** Before changes, at 1280 × 820, switching Convert → Info changes the shell's grid rows from `820px 0px` to `596.672px 223.328px`; the second row belongs to Sonner's notification section. The navigation bottom becomes 596.672px instead of 820px.

Windows package execution requires Windows; validate shared assets and configuration locally and state that limit explicitly. Do not publish or push without a user request.
