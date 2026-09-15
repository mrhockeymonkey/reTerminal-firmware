//! Browser preview backend (design brief §9): implements `DrawTarget` by
//! writing into an in-memory pixel buffer, then blits it to an HTML
//! `<canvas>` via `wasm-bindgen`/`web-sys`. Compiled to
//! `wasm32-unknown-unknown` for actual use; also checked against the host
//! target in CI/dev sessions since it has no wasm-only dependencies yet.
