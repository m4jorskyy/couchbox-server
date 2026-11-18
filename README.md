# CouchBox Server (Rust + Tauri v2 Backend)

Backend and desktop shell for **CouchBox** (Laptop Smart TV + Phone Remote).
Pairs with the frontend repository at `~/WebstormProjects/couchbox-web`.

## Modules (`src/`)

- `lib.rs` / `main.rs` — Tauri v2 entrypoint, managed state, and `#[tauri::command]` handlers.
- `server.rs` — Axum HTTP/WebSocket server (`0.0.0.0:47832`), QR/PIN pairing API, and embedded Phone Remote PWA (`rust-embed`).
- `input.rs` — Dedicated OS thread using `enigo` for D-pad, trackpad, phone keyboard typing, media keys, and volume.
- `launcher.rs` — Chrome / Edge / Brave `--kiosk` launcher with dedicated TV profile (`CouchBox/tv-browser-profile`) and Smart TV User-Agent for YouTube TV.
- `power.rs` — Cross-platform screen-off & laptop sleep actions.
- `state.rs` & `protocol.rs` — Persistent JSON config (trusted tokens, custom apps) and shared WebSocket protocol.

## Running in Dev (with `couchbox-web`)

```bash
npm install
npm run dev
```

Or directly from **RustRover** after building the frontend once (`npm run build` inside `~/WebstormProjects/couchbox-web`):
- Run `cargo run` (or `cargo build --release`).

## Building a Windows Installer (`.exe` / `.msi`)

On a Windows machine (or via `.github/workflows/build-windows.yml`):
```bash
npm install
npm run build
```
Output binaries will be in `target/release/couchbox.exe` and `target/release/bundle/nsis/`.
