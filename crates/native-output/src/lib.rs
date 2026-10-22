//! Native numerical writers and protected result-directory publication, without HDF5.
//! Low-level writers require a trusted stem and exclusively owned scratch directory;
//! adapters publish their complete file groups through [`directory::publish`].
pub mod bellhop;
pub mod diagnostic;
pub mod directory;
pub mod kraken;
mod native;

pub use native::create;
