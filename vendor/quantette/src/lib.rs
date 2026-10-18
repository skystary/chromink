//! A library for fast and high quality image quantization and palette generation.
//!
//! To get started, see the [`Pipeline`] builder struct which serves as the high-level API.
//! Otherwise, manually compose functions from the other exported modules to suit your needs.
//!
//! # Features
//!
//! To reduce dependencies and compile times, `quantette` has several `cargo` features
//! that can be turned off or on:
//! - `kmeans`: adds an additional high quality quantization method that takes longer to run.
//! - `threads`: exposes parallel versions of most functions via [`rayon`].
//! - `image`: enables integration with the [`image`] crate.
//! - `std`: use Rust's standard library (this crate is `no_std` compatible if `threads` and `image` are disabled).
//!
//! By default, all features are enabled.
//!
//! All of the color types present in the public API for this crate
//! (like [`Srgb`](palette::Srgb) or [`Oklab`](palette::Oklab)) are from the [`palette`] crate.
//! You can check it out for more information. For example, its documentation
//! should provide you everything you need to know to [cast](palette::cast)
//! a `Vec<Srgb<u8>>` into a `Vec<[u8; 3]>` or vice versa.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unreachable,
    clippy::panic,
    clippy::exit,
    clippy::unused_result_ok,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs
)]
#![cfg_attr(not(any(feature = "std", test)), no_std)]

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod color_map;
pub mod color_space;
pub mod deps;
pub mod dither;

mod api;
mod quantize;
mod traits;
mod types;

pub use api::*;
pub use quantize::*;
pub use traits::*;
pub use types::*;

pub(crate) use color_map::IndexedColorMap;

/// The maximum amount of pixels supported by this library ([`u32::MAX`]).
///
/// This applies to all image types as well as pixel slices or buffers.
pub const MAX_PIXELS: u32 = u32::MAX;
