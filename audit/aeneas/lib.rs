//! Translation harness for the production journal codec.
//!
//! The module path deliberately points at the file compiled by the production crate. Charon and
//! Aeneas therefore translate the same Rust definitions instead of a verification-only copy.

#![allow(dead_code)]

#[path = "../../src/journal.rs"]
mod journal;
