use crate::value::Value;

/// Local execution frame — up to 256 slots indexed by u8.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    slots: Vec<Option<Value>>,
}

impl Frame {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Grow the slot vector if needed and set a value.
    pub fn set(&mut self, index: u8, value: Value) {
        let idx = index as usize;
        if idx >= self.slots.len() {
            self.slots.resize(idx + 1, None);
        }
        self.slots[idx] = Some(value);
    }

    pub fn get(&self, index: u8) -> Option<&Value> {
        self.slots.get(index as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, index: u8) -> Option<&mut Value> {
        self.slots.get_mut(index as usize)?.as_mut()
    }

    pub fn remove(&mut self, index: u8) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            *slot = None;
        }
    }

    /// Snapshot a clone of all slots for TRY rollback.
    pub fn snapshot(&self) -> Vec<Option<Value>> {
        self.slots.clone()
    }

    /// Restore from a snapshot.
    pub fn restore(&mut self, snap: Vec<Option<Value>>) {
        self.slots = snap;
    }
}

/// Transient execution frame for TMP blocks (t0-t15).
#[derive(Debug, Clone)]
pub struct TmpFrame {
    pub slots: [Option<Value>; 16],
}
impl Default for TmpFrame {
    fn default() -> Self {
        Self {
            slots: Default::default(),
        }
    }
}
impl TmpFrame {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Global register — unlimited slots indexed by u32.
#[derive(Debug, Clone, Default)]
pub struct GlobalRegister {
    slots: Vec<Option<Value>>,
}

impl GlobalRegister {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    pub fn set(&mut self, index: u32, value: Value) {
        let idx = index as usize;
        if idx >= self.slots.len() {
            self.slots.resize(idx + 1, None);
        }
        self.slots[idx] = Some(value);
    }

    pub fn get(&self, index: u32) -> Option<&Value> {
        self.slots.get(index as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, index: u32) -> Option<&mut Value> {
        self.slots.get_mut(index as usize)?.as_mut()
    }

    pub fn remove(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            *slot = None;
        }
    }

    pub fn snapshot(&self) -> Vec<Option<Value>> {
        self.slots.clone()
    }

    pub fn restore(&mut self, snap: Vec<Option<Value>>) {
        self.slots = snap;
    }
}
