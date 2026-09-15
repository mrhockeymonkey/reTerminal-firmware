//! Shared "screen spec" content contract (design brief §8).
//!
//! Serde types for the JSON document that flows: `server` (holds/serves it)
//! -> `firmware` (fetches and renders it) -> `web-preview` (renders the same
//! thing in a browser). Kept `no_std` so it can be used unmodified from the
//! ESP32-S3 firmware; `server` and `web-preview` use it from a `std` context
//! without any extra work.
//!
//! Empty until the v1 schema (region model, text styles, alignment) is
//! finalized — see the open questions in the design brief.
#![no_std]
