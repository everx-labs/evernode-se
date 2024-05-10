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

use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use ever_block::{Message, Serializable};

/// This FIFO accumulates inbound messages from all types of receivers.
/// The struct might be used from many threads. It provides internal mutability.
pub struct InMessagesQueue {
    present: Condvar,
    storage: Mutex<VecDeque<Message>>,
    capacity: usize,
    stopped: std::sync::atomic::AtomicBool,
}

#[allow(dead_code)]
impl InMessagesQueue {
    /// Create new instance of InMessagesQueue.
    pub fn new(capacity: usize) -> Self {
        InMessagesQueue {
            present: Condvar::new(),
            storage: Mutex::new(Default::default()),
            capacity,
            stopped: Default::default(),
        }
    }

    /// Include message into end queue.
    pub fn queue(&self, msg: Message) -> Result<(), Option<Message>> {
        if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(None);
        }
        
        let mut storage = self.storage.lock();
        if storage.len() >= self.capacity {
            return Err(Some(msg));
        }

        log::debug!(target: "node", "Queued message: {:?}", msg);
        storage.push_back(msg);
        self.present.notify_one();

        Ok(())
    }

    /// Extract oldest message for specified workchain from queue.
    pub fn dequeue(&self, workchain_id: i32) -> Option<Message> {
        let mut storage = self.storage.lock();
        for i in 0..storage.len() {
            if storage[i].workchain_id() == Some(workchain_id) {
                return storage.remove(i);
            }
        }
        None
    }

    pub fn print_message(msg: &Message) {
        log::info!("message: {:?}", msg);
        if let Ok(cell) = msg.serialize() {
            if let Ok(data) = ever_block::write_boc(&cell) {
                std::fs::create_dir_all("export").ok();
                std::fs::write(&format!("export/msg_{:x}", cell.repr_hash()), &data).ok();
            }
        }
    }

    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    pub fn wait_new_message(&self) -> Result<(), ()> {
        if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(());
        }

        let mut mutex_guard = self.storage.lock();
        self.present.wait(&mut mutex_guard);
        
        if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
            Err(())
        } else {
            Ok(())
        }
    }

    pub fn is_empty(&self) -> bool {
        self.storage.lock().is_empty()
    }

    /// The length of queue.
    pub fn len(&self) -> usize {
        self.storage.lock().len()
    }

    pub fn stop(&self) {
        self.stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        self.present.notify_one();
    }

    pub fn start(&self) {
        self.stopped.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
