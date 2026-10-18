# Native terminal acceptance

This test plan implements the pinned terminal contract; it does not redefine it.
The immutable pre-change v0.1.12 captures include the exact old production renderer,
all four generated entrypoints and existing-root/legacy migration delegates. Keep
raw VT and source hashes alongside the canonical owner evidence.

Before accepting a candidate, reconstruct each captured PTY/ConPTY chunk using
pyte, inspect the final frames, and assert that every active frame uses two rows
at a fixed position between explicit output/prompt boundaries. Stage changes and
spinner frames must never increase the number of nonempty activity rows. Assert
at least two spinner glyphs with unchanged bytes, 0–99 pending verification, no
percent for unknown lengths, shorter label tail clearing and width after resize.

Exercise generated install/update on native PS5.1 and POSIX sh (both curl/wget),
including existing root, legacy migration and no-install refusal. Cover download
failure, hash mismatch, cancellation in each phase, confirmation accept/refuse/EOF,
child status 0/7/130, stdout-only/stderr-only redirection, CI, dumb, plain, off, JSON
and NO_COLOR. Use private homes and task-owned processes. Capture actual Windows
Terminal and VS Code screenshots as well as native trace reconstructions. Source
fixture tests do not establish actual release upgrade/rollback acceptance.

An unavailable native target is unverified. Native macOS/Windows claims require
that target's real runtime; Linux simulation does not satisfy them. Publication
remains gated until required evidence and independent review are complete.
