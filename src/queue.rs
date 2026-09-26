use crate::options::MAX_QUEUE_BYTES;
use amitoki_relay::{Delivery, Frame, Receipt, RelayError};
use std::collections::{HashMap, HashSet, VecDeque};

// ACK後の再送は直近65536件まで重複除去する。未ACKのフレームは追い出さない。
const COMPLETED_CAPACITY: usize = 65536;
pub struct Queue {
    capacity: usize,
    bytes: usize,
    pending: HashMap<String, Frame>,
    order: VecDeque<String>,
    completed: HashSet<String>,
    completed_order: VecDeque<String>,
}
impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            bytes: 0,
            pending: HashMap::new(),
            order: VecDeque::new(),
            completed: HashSet::new(),
            completed_order: VecDeque::new(),
        }
    }
    pub fn accept(&mut self, sender: &str, frames: Vec<Frame>) -> Result<(), RelayError> {
        let mut additions: Vec<(String, Frame)> = Vec::new();
        for frame in frames {
            frame.validate()?;
            let receipt = format!("{sender}/{}", frame.id);
            if let Some(previous) = self.pending.get(&receipt).or_else(|| additions.iter().find(|(key, _)| key == &receipt).map(|(_, frame)| frame)) {
                if previous != &frame {
                    return Err(RelayError::permanent("同じフレームIDに異なる内容が届きました"));
                }
            } else if !self.completed.contains(&receipt) {
                additions.push((receipt, frame));
            }
        }
        let bytes: usize = additions.iter().map(|(_, frame)| frame.bytes.len()).sum();
        if self.pending.len() + additions.len() > self.capacity || self.bytes + bytes > MAX_QUEUE_BYTES {
            return Err(RelayError::retryable("P2Pの受信キューが満杯です"));
        }
        for (receipt, frame) in additions {
            self.order.push_back(receipt.clone());
            self.pending.insert(receipt, frame);
        }
        self.bytes += bytes;
        Ok(())
    }
    pub fn receive(&self, limit: usize) -> Vec<Delivery> {
        self.order
            .iter()
            .take(limit)
            .map(|receipt| Delivery {
                frame: self.pending[receipt].clone(),
                receipt: Receipt(receipt.clone()),
            })
            .collect()
    }
    pub fn acknowledge(&mut self, receipts: &[Receipt]) {
        for receipt in receipts {
            if let Some(frame) = self.pending.remove(&receipt.0) {
                self.bytes -= frame.bytes.len();
                self.completed.insert(receipt.0.clone());
                self.completed_order.push_back(receipt.0.clone());
            }
        }
        self.order.retain(|receipt| self.pending.contains_key(receipt));
        while self.completed_order.len() > COMPLETED_CAPACITY {
            if let Some(receipt) = self.completed_order.pop_front() {
                self.completed.remove(&receipt);
            }
        }
    }
}
