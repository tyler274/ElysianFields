//! Executable entry point for M1 web delivery native and Wasm build/size qualification.

use m1_web_delivery::graphics::BrowserGraphicsCapabilities;
use m1_web_delivery::run_headless_delivery_probe;

/// Exported C-ABI probe entry point callable from strict-CSP browser harnesses on Wasm.
#[unsafe(no_mangle)]
pub extern "C" fn elysian_m1_delivery_probe(webgpu_available: u32, webgl2_available: u32) -> u32 {
    use m1_web_delivery::graphics::{WebGl2Availability, WebGpuAvailability};

    let capabilities = BrowserGraphicsCapabilities {
        webgpu: if webgpu_available != 0 {
            WebGpuAvailability::Available
        } else {
            WebGpuAvailability::Unavailable
        },
        webgl2: if webgl2_available != 0 {
            WebGl2Availability::Available
        } else {
            WebGl2Availability::Unavailable
        },
    };
    run_headless_delivery_probe(capabilities)
}

fn main() {
    let exit_code = run_headless_delivery_probe(BrowserGraphicsCapabilities::webgl2_only());
    assert_eq!(
        exit_code, 0,
        "canonical M1 web delivery probe must report ready"
    );
}
