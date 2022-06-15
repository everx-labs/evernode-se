/*
* Copyright 2018-2022 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.  You may obtain a copy of the
* License at:
*
* https://www.ton.dev/licenses
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and limitations
* under the License.
*/

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
