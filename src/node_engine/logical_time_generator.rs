use std::sync::Arc;
use parking_lot::Mutex;

///
/// Struct LogicalTime Generator
///
pub struct LogicalTimeGenerator {
    current: Arc<Mutex<u64>>,
}

impl Default for LogicalTimeGenerator {
    fn default() -> Self {
        LogicalTimeGenerator {
            current: Arc::new(Mutex::new(0)),
        }
    }
}

///
/// Implementation of Logical Time Generator
///
impl LogicalTimeGenerator {
    ///
    /// Initialize new instance with current value
    ///
    pub fn with_init_value(current: u64) -> Self {
        LogicalTimeGenerator {
            current: Arc::new(Mutex::new(current)),
        }
    }

    ///
    /// Get next value of logical time
    ///
    pub fn get_next_time(&mut self) -> u64 {
        let mut current = self.current.lock();
        *current += 1;
        *current
    }

    ///
    /// Get current value of logical time
    ///
    pub fn get_current_time(&self) -> u64 {
        let current = self.current.lock();
        *current
    }
}
