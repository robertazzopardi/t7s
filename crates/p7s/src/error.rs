//! Fatal / startup results. Recoverable failures go to the status line.

use core::{error::Error, result::Result as StdResult};

pub type Result<T, E = Box<dyn Error + Send + Sync>> = StdResult<T, E>;
