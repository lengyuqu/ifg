//! Typed REST client for the IronForge API (`/api/v1/`).
//!
//! The HTTP layer, error model, pagination and resource accessors land in M1;
//! see `docs/architecture/ifg-project-design-2026-11.md` §4.

mod secret;

pub use secret::Secret;
