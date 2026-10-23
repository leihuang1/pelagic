//! BELLHOP and KRAKEN HDF5 adapters with separate, versioned schemas.

pub mod bellhop;
use native_output::{bellhop as bellhop_native, kraken as kraken_native};
pub use native_output::{diagnostic, directory};
mod hdf5;
pub mod kraken;
mod publication;
