use std::sync::atomic::{AtomicU8, Ordering};
#[derive(Debug, PartialEq)]
pub enum ExitDecision {
    Begin,
    Wait,
    Allow,
}
#[derive(Default)]
pub struct ShutdownGate(AtomicU8);
impl ShutdownGate {
    pub fn request(&self) -> ExitDecision {
        match self
            .0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => ExitDecision::Begin,
            Err(2) => ExitDecision::Allow,
            Err(_) => ExitDecision::Wait,
        }
    }
    pub fn cleaned(&self) {
        self.0.store(2, Ordering::SeqCst);
    }
    pub fn retry(&self) {
        self.0.store(0, Ordering::SeqCst);
    }
}
