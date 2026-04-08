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
#[derive(Debug, Clone, Default)]
pub struct TmpFrame {
    pub slots: [Option<Value>; 16],
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Frame tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_frame_set_and_get() {
        let mut frame = Frame::new();
        frame.set(0, Value::Int32(42));
        frame.set(5, Value::Bool(true));
        assert_eq!(frame.get(0), Some(&Value::Int32(42)));
        assert_eq!(frame.get(5), Some(&Value::Bool(true)));
        assert_eq!(frame.get(3), None);
    }

    #[test]
    fn test_frame_set_overwrites() {
        let mut frame = Frame::new();
        frame.set(2, Value::Int32(1));
        frame.set(2, Value::Int32(99));
        assert_eq!(frame.get(2), Some(&Value::Int32(99)));
    }

    #[test]
    fn test_frame_get_beyond_allocated_returns_none() {
        let frame = Frame::new();
        assert_eq!(frame.get(200), None);
    }

    #[test]
    fn test_frame_remove_clears_slot() {
        let mut frame = Frame::new();
        frame.set(1, Value::Uint8(7));
        frame.remove(1);
        assert_eq!(frame.get(1), None);
    }

    #[test]
    fn test_frame_remove_unset_slot_is_safe() {
        let mut frame = Frame::new();
        frame.remove(10); // no-op on empty frame — must not panic
    }

    #[test]
    fn test_frame_snapshot_and_restore() {
        let mut frame = Frame::new();
        frame.set(0, Value::Int32(10));
        frame.set(1, Value::Bool(false));
        let snap = frame.snapshot();

        frame.set(0, Value::Int32(99));
        frame.set(2, Value::Float32(1.5));
        assert_eq!(frame.get(0), Some(&Value::Int32(99)));

        frame.restore(snap);
        assert_eq!(frame.get(0), Some(&Value::Int32(10)));
        assert_eq!(frame.get(1), Some(&Value::Bool(false)));
        assert_eq!(frame.get(2), None);
    }

    #[test]
    fn test_frame_get_mut() {
        let mut frame = Frame::new();
        frame.set(3, Value::Int32(0));
        if let Some(Value::Int32(n)) = frame.get_mut(3) {
            *n = 55;
        }
        assert_eq!(frame.get(3), Some(&Value::Int32(55)));
    }

    // ── GlobalRegister tests ──────────────────────────────────────────────────

    #[test]
    fn test_global_set_and_get() {
        let mut reg = GlobalRegister::new();
        reg.set(0, Value::Uint64(1_000_000));
        reg.set(1000, Value::Bool(true));
        assert_eq!(reg.get(0), Some(&Value::Uint64(1_000_000)));
        assert_eq!(reg.get(1000), Some(&Value::Bool(true)));
        assert_eq!(reg.get(500), None);
    }

    #[test]
    fn test_global_remove() {
        let mut reg = GlobalRegister::new();
        reg.set(5, Value::Int8(-1));
        reg.remove(5);
        assert_eq!(reg.get(5), None);
    }

    #[test]
    fn test_global_snapshot_and_restore() {
        let mut reg = GlobalRegister::new();
        reg.set(0, Value::Int32(1));
        let snap = reg.snapshot();

        reg.set(0, Value::Int32(999));
        reg.restore(snap);
        assert_eq!(reg.get(0), Some(&Value::Int32(1)));
    }

    #[test]
    fn test_global_get_mut() {
        let mut reg = GlobalRegister::new();
        reg.set(2, Value::Uint32(0));
        if let Some(Value::Uint32(n)) = reg.get_mut(2) {
            *n = 42;
        }
        assert_eq!(reg.get(2), Some(&Value::Uint32(42)));
    }

    // ── TmpFrame tests ────────────────────────────────────────────────────────

    #[test]
    fn test_tmp_frame_default_all_none() {
        let tf = TmpFrame::new();
        for slot in &tf.slots {
            assert!(slot.is_none());
        }
    }
}
