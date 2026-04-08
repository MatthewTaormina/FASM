/// Controls the instruction execution rate for a sandbox.
/// 0 = unlimited (no throttle).
pub struct ClockController {
    pub instructions_per_tick: u64,
    pub ticks_executed: u64,
    pub instructions_executed: u64,
}

impl ClockController {
    pub fn new() -> Self {
        Self {
            instructions_per_tick: 0,
            ticks_executed: 0,
            instructions_executed: 0,
        }
    }

    /// Returns true if the sandbox may execute another instruction this tick.
    pub fn can_execute(&self) -> bool {
        if self.instructions_per_tick == 0 { return true; }
        self.instructions_executed < self.instructions_per_tick
    }

    pub fn tick_instruction(&mut self) {
        self.instructions_executed += 1;
    }

    pub fn end_tick(&mut self) {
        self.ticks_executed += 1;
        self.instructions_executed = 0;
    }
}

impl Default for ClockController {
    fn default() -> Self { Self::new() }
}
