mod classify;
mod diagnostics;
mod resolve;

pub(super) use classify::classify_resource_shape;
pub use diagnostics::KfxResourceDiagnostic;
pub(super) use resolve::{
    normalize_kfx_resource_path, resolve_native_resource_paths, validate_resource_resolution,
    NativeResourcePathResolution, ResourceBinding,
};
