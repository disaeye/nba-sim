pub const HTML: &str = include_str!("../static/index.html");
pub const CSS: &str = include_str!("../static/style.css");
pub const JS: &str = include_str!("../static/app.js");
pub const WASM_JS: &str = include_str!("../static/wasm/nba_wasm.js");
pub const WASM_BIN: &[u8] = include_bytes!("../static/wasm/nba_wasm_bg.wasm");
