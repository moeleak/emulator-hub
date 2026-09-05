//! Google Android Emulator process and authenticated gRPC integration.
mod apk;
mod controller;
pub mod provision;
mod sdk;
pub use apk::{inspect_apk_abis, validate_apk_abi};
pub use controller::*;
pub use sdk::prepare_runtime_sdk;

#[allow(clippy::all)]
pub mod proto {
    tonic::include_proto!("android.emulation.control");
}
