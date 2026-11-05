//! Pure verification lanes (L0). Effectful lanes (L1/L3) live in `aa-exec::lanes`.

pub mod static_lane;

pub use static_lane::run_l0;
