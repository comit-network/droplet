mod utils;
use log::{debug, info};

use crate::utils::set_panic_hook;
use wasm_bindgen::prelude::*;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen(start)]
pub fn setup_lib() {
    set_panic_hook();
    wasm_logger::init(wasm_logger::Config::default());
    debug!("Wasm lib initialized");
}

#[wasm_bindgen]
pub fn hello(name: &str) -> String {
    let string = format!("Hello {}", name);
    info!("Logging in rust: {}", string);
    string
}
