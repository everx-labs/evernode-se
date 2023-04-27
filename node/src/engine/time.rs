use ton_block::UnixTime32;

#[derive(Copy, Clone, Debug)]
pub enum BlockTimeMode {
    System = 0,
    Seq = 1,
}

impl BlockTimeMode {
    pub fn is_seq(&self) -> bool {
        if let Self::Seq = self {
            true
        } else {
            false
        }
    }
}

impl Default for BlockTimeMode {
    fn default() -> Self {
        Self::System
    }
}

pub struct BlockTime {
    pub delta: u32,
    pub last_time: u32,
    pub mode: BlockTimeMode,
    pub seq_mode_interval: u32,
}

impl BlockTime {
    pub(crate) fn new() -> Self {
        Self {
            delta: 0,
            mode: BlockTimeMode::System,
            seq_mode_interval: 1,
            last_time: 1,
        }
    }

    pub fn increase_delta(&mut self, delta: u32) {
        self.delta += delta;
        log::info!(target: "node", "SE time delta set to {}", self.delta);
    }

    pub fn reset_delta(&mut self) {
        self.delta = 0;
        log::info!(target: "node", "SE time delta set to 0");
    }

    pub fn set_mode(&mut self, mode: BlockTimeMode) {
        self.mode = mode;
        log::info!(target: "node", "SE seq mode to {:?}", mode);
    }

    pub fn get_next(&self) -> u32 {
        match self.mode {
            BlockTimeMode::System => UnixTime32::now().as_u32() + self.delta,
            BlockTimeMode::Seq => self.last_time + self.seq_mode_interval + self.delta,
        }
    }

    pub fn set_last(&mut self, time: u32) {
        self.last_time = time.checked_sub(self.delta).unwrap_or(0);
    }
}
