# Windows no-install ZIP plan

**Goal:** Distribute a Windows x64 ZIP that users extract and launch by double-clicking FrameEase.exe.

**Design:** Stage the existing release executable with the complete FFmpeg folder, optional WebView2 loader, license notices, and bilingual instructions. Use Windows' archive tools to package it. Preserve the application's existing data locations; this request concerns installation, not moving settings and queues to removable storage. The default package uses the system WebView2 Runtime. A self-contained runtime variant is an optional user preference, pending clarification.

- [ ] Add filesystem-based staging tests for complete resources, missing dependencies, and repeatable clean output.
- [ ] Add the staging helper and Windows ZIP packaging command.
- [ ] Verify ZIP extraction in a Unicode/spaced path, exact resource hashes, FFmpeg execution without PATH dependencies, and native GUI startup/backend detection from a different working directory on Windows CI.
- [ ] Add the ZIP to CI artifacts and release uploads/checksums; document extraction, runtime dependency, and local data locations.
- [ ] Run relevant tests and review the final change before delivery.

Scope excludes a web application and single-file self-extracting launchers. Existing NSIS/MSI distribution remains available alongside the ZIP. Native Windows acceptance must come from Windows CI; macOS-only staging tests do not prove the application launches on Windows.
