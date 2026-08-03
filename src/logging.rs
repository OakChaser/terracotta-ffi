use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const CAPACITY: usize = 512;

static RING: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

pub fn log(line: &str) {
    #[cfg(debug_assertions)]
    eprintln!("{}", line);

    if let Ok(mut ring) = RING.get_or_init(|| Mutex::new(VecDeque::new())).lock() {
        if ring.len() >= CAPACITY {
            ring.pop_front();
        }
        ring.push_back(line.to_string());
    }
}

pub fn recent(limit: usize) -> Vec<String> {
    let ring = RING.get_or_init(|| Mutex::new(VecDeque::new()));
    let guard = ring.lock().unwrap();
    guard.iter().rev().take(limit).cloned().rev().collect()
}
