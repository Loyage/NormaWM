//! 独立的人类控制面入口。

use normawm::{error::NormaError, monitor, runtime};

fn main() -> Result<(), NormaError> {
    runtime::init_tracing();
    monitor::run_control_panel().map_err(NormaError::WinitBackend)
}
