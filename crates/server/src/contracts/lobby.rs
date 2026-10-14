//! Authenticated recipients and task outcomes shared by lobby services.
use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identity {
    pub title: u32,
    pub user: u64,
    pub seed: u32,
}

pub struct DirectedPush {
    pub target: Identity,
    pub body: Vec<u8>,
}
pub struct TaskResult {
    pub label: &'static str,
    pub result: Result<Vec<Vec<u8>>, u32>,
    /// Unsolicited lobby kind-2 bodies, sent after the task acknowledgement.
    pub pushes: Vec<Vec<u8>>,
    /// Plaintext notifications for other authenticated connections.
    pub deliveries: Vec<DirectedPush>,
}
impl TaskResult {
    pub(crate) fn success(label: &'static str, records: Vec<Vec<u8>>) -> Self {
        Self {
            label,
            result: Ok(records),
            pushes: Vec::new(),
            deliveries: Vec::new(),
        }
    }
    pub(crate) fn failure(label: &'static str, code: u32) -> Self {
        Self {
            label,
            result: Err(code),
            pushes: Vec::new(),
            deliveries: Vec::new(),
        }
    }
    pub(crate) fn push_to(&mut self, actor: Identity, target: Identity, body: Vec<u8>) {
        if actor == target {
            self.pushes.push(body);
        } else {
            self.deliveries.push(DirectedPush { target, body });
        }
    }
}
