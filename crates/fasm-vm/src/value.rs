use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// The runtime value type for every FASM slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    Uint8(u8),
    Uint16(u16),
    Uint32(u32),
    Uint64(u64),
    Float32(f32),
    Float64(f64),
    /// Mutable reference: (is_global, index)
    RefMut(bool, u32),
    /// Immutable reference: (is_global, index)
    RefImm(bool, u32),
    // ── Core collections ────────────────────────────────────────────────────
    Vec(FasmVec),
    Struct(FasmStruct),
    Stack(FasmStack),
    Queue(FasmQueue),
    HeapMin(FasmHeapMin),
    HeapMax(FasmHeapMax),
    // ── High-performance collections ─────────────────────────────────────────
    /// FxHashMap<u32, Value> — O(1) average integer-keyed sparse array.
    /// Uses FxHash (same hasher as rustc) — non-cryptographic, optimized for integer keys.
    Sparse(FasmSparse),
    /// BTreeMap<u32, Value> — O(log n) ordered integer-keyed map.
    /// Provides stable ordered iteration; use BTREE_MIN / BTREE_MAX for range traversal.
    BTree(FasmBTree),
    /// Read-only copied sub-range of a VEC. Produced by VEC_SLICE.
    /// Immutable — writes fault with TypeMismatch. GET_IDX and LEN are O(1).
    Slice(FasmSlice),
    /// Double-ended queue — O(1) push/pop from both ends via VecDeque.
    Deque(FasmDeque),
    /// Bit-addressable boolean array backed by Vec<u8>. LEN returns bits.
    Bitset(FasmBitset),
    /// Arbitrary-width bit field storage. BITVEC_READ/WRITE access N-bit fields.
    Bitvec(FasmBitvec),
    // ── Wrapper types ─────────────────────────────────────────────────────────
    Option(Box<FasmOption>),
    Result(Box<FasmResult>),
    /// Future: resolved value (None = pending)
    Future(Option<Box<Value>>),
}

// ─── Core collection types ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmVec(pub Vec<Value>);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmStruct(pub Vec<(u32, Value)>);

impl FasmStruct {
    pub fn get(&self, key: &u32) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: u32, val: Value) {
        if let Some(existing) = self.0.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = val;
        } else {
            self.0.push((key, val));
        }
    }

    pub fn contains_key(&self, key: &u32) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    pub fn remove(&mut self, key: &u32) {
        if let Some(idx) = self.0.iter().position(|(k, _)| k == key) {
            self.0.remove(idx);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmStack(pub Vec<Value>);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmQueue(pub VecDeque<Value>);

/// Min-heap: we store values as ordered wrappers. Requires Value: Ord.
/// For simplicity we store them internally as a sorted Vec and heapify.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmHeapMin(pub Vec<Value>);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmHeapMax(pub Vec<Value>);

// ─── High-performance collection types ───────────────────────────────────────

/// Sparse array backed by FxHashMap<u32, Value>.
/// FxHash has zero per-insertion DoS protection overhead — ideal for integer keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FasmSparse(pub FxHashMap<u32, Value>);

// Manual PartialEq: FxHashMap is HashMap underneath, which implements PartialEq.
impl PartialEq for FasmSparse {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl FasmSparse {
    #[inline]
    pub fn get(&self, key: u32) -> Option<&Value> {
        self.0.get(&key)
    }
    #[inline]
    pub fn insert(&mut self, key: u32, val: Value) {
        self.0.insert(key, val);
    }
    #[inline]
    pub fn remove(&mut self, key: u32) {
        self.0.remove(&key);
    }
    #[inline]
    pub fn contains_key(&self, key: u32) -> bool {
        self.0.contains_key(&key)
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Ordered map backed by BTreeMap<u32, Value>.
/// Provides O(log n) get/set/del and O(1) min/max key access via tree structure.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmBTree(pub BTreeMap<u32, Value>);

impl FasmBTree {
    #[inline]
    pub fn get(&self, key: u32) -> Option<&Value> {
        self.0.get(&key)
    }
    #[inline]
    pub fn insert(&mut self, key: u32, val: Value) {
        self.0.insert(key, val);
    }
    #[inline]
    pub fn remove(&mut self, key: u32) {
        self.0.remove(&key);
    }
    #[inline]
    pub fn contains_key(&self, key: u32) -> bool {
        self.0.contains_key(&key)
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// O(log n) — BTree keeps smallest key at leftmost node.
    #[inline]
    pub fn min_key(&self) -> Option<u32> {
        self.0.keys().next().copied()
    }
    /// O(log n) — BTree keeps largest key at rightmost node.
    #[inline]
    pub fn max_key(&self) -> Option<u32> {
        self.0.keys().next_back().copied()
    }
}

/// Read-only copied sub-range view of a VEC.
/// Created by VEC_SLICE — O(len) copy at creation, O(1) GET_IDX thereafter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmSlice(pub Vec<Value>);

impl FasmSlice {
    #[inline]
    pub fn get(&self, idx: usize) -> Option<&Value> {
        self.0.get(idx)
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Double-ended queue — O(1) push/pop from both ends.
/// Supports PUSH (append), PREPEND (push_front), DEQUEUE (pop_front), POP_BACK, PEEK, PEEK_BACK.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmDeque(pub VecDeque<Value>);

/// Bit-addressable boolean array backed by Vec<u8>.
/// Bit `i` lives in byte `i/8`, at bit position `i%8` (LSB-first within each byte).
/// LEN returns the logical bit count, not the byte count.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmBitset {
    pub bytes: Vec<u8>,
    pub len: u32, // logical length in bits
}

impl FasmBitset {
    pub fn new(len_bits: u32) -> Self {
        let n_bytes = len_bits.div_ceil(8) as usize;
        Self {
            bytes: vec![0u8; n_bytes],
            len: len_bits,
        }
    }

    /// Set bit at index (auto-grows if necessary).
    #[inline]
    pub fn set_bit(&mut self, idx: u32) {
        let byte_idx = (idx / 8) as usize;
        if byte_idx >= self.bytes.len() {
            self.grow_to(idx + 1);
        }
        self.bytes[byte_idx] |= 1 << (idx % 8);
    }

    /// Clear bit at index. No-op if beyond current length.
    #[inline]
    pub fn clr_bit(&mut self, idx: u32) {
        let byte_idx = (idx / 8) as usize;
        if byte_idx < self.bytes.len() {
            self.bytes[byte_idx] &= !(1 << (idx % 8));
        }
    }

    /// Get bit value. Returns false for out-of-range indices.
    #[inline]
    pub fn get_bit(&self, idx: u32) -> bool {
        let byte_idx = (idx / 8) as usize;
        if byte_idx < self.bytes.len() {
            (self.bytes[byte_idx] >> (idx % 8)) & 1 == 1
        } else {
            false
        }
    }

    /// Toggle bit at index (auto-grows if necessary).
    #[inline]
    pub fn flip_bit(&mut self, idx: u32) {
        let byte_idx = (idx / 8) as usize;
        if byte_idx >= self.bytes.len() {
            self.grow_to(idx + 1);
        }
        self.bytes[byte_idx] ^= 1 << (idx % 8);
    }

    /// Count set bits (hardware popcount via Rust's u8::count_ones).
    #[inline]
    pub fn popcount(&self) -> u32 {
        self.bytes.iter().map(|b| b.count_ones()).sum()
    }

    /// Extend the bitset to at least `new_len_bits` capacity (zero-fill new bits).
    pub fn grow_to(&mut self, new_len_bits: u32) {
        let new_byte_count = new_len_bits.div_ceil(8) as usize;
        if new_byte_count > self.bytes.len() {
            self.bytes.resize(new_byte_count, 0);
        }
        if new_len_bits > self.len {
            self.len = new_len_bits;
        }
    }
}

/// Arbitrary-width bit field storage backed by Vec<u8>.
/// Bits are packed LSB-first within each byte (same layout as BITSET).
/// BITVEC_READ/WRITE can access fields up to 64 bits wide.
/// Good for non-byte-aligned encodings: 4-bit nibbles, 12-bit audio samples, etc.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FasmBitvec {
    pub bytes: Vec<u8>,
    pub bit_len: u64, // total bits logically stored
}

impl FasmBitvec {
    /// Read up to 64 consecutive bits starting at `bit_start`. Returns 0 for out-of-range.
    pub fn read_bits(&self, bit_start: u64, bit_len: u32) -> u64 {
        let bit_len = (bit_len as u64).min(64);
        let mut result = 0u64;
        for i in 0..bit_len {
            let pos = bit_start + i;
            let byte_idx = (pos / 8) as usize;
            let bit_pos = (pos % 8) as u32;
            if byte_idx < self.bytes.len() {
                let bit = ((self.bytes[byte_idx] >> bit_pos) & 1) as u64;
                result |= bit << i;
            }
        }
        result
    }

    /// Write up to 64 bits from `value` starting at `bit_start`. Auto-grows as needed.
    pub fn write_bits(&mut self, bit_start: u64, bit_len: u32, value: u64) {
        let bit_len = (bit_len as u64).min(64);
        let end_bit = bit_start + bit_len;
        let needed_bytes = end_bit.div_ceil(8) as usize;
        if needed_bytes > self.bytes.len() {
            self.bytes.resize(needed_bytes, 0);
        }
        if end_bit > self.bit_len {
            self.bit_len = end_bit;
        }
        for i in 0..bit_len {
            let pos = bit_start + i;
            let byte_idx = (pos / 8) as usize;
            let bit_pos = (pos % 8) as u32;
            let v = ((value >> i) & 1) as u8;
            if v == 1 {
                self.bytes[byte_idx] |= 1 << bit_pos;
            } else {
                self.bytes[byte_idx] &= !(1 << bit_pos);
            }
        }
    }

    /// Append `bit_len` bits from `value` (LSB-first) at the current end.
    #[inline]
    pub fn push_bits(&mut self, value: u64, bit_len: u32) {
        let start = self.bit_len;
        self.write_bits(start, bit_len, value);
    }
}

// ─── wrapper types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FasmOption {
    None,
    Some(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FasmResult {
    Ok(Value),
    Err(u32), // fault code
}

// ─── helpers ─────────────────────────────────────────────────────────────────

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int8(n) => *n != 0,
            Value::Int16(n) => *n != 0,
            Value::Int32(n) => *n != 0,
            Value::Int64(n) => *n != 0,
            Value::Uint8(n) => *n != 0,
            Value::Uint16(n) => *n != 0,
            Value::Uint32(n) => *n != 0,
            Value::Uint64(n) => *n != 0,
            Value::Float32(f) => *f != 0.0,
            Value::Float64(f) => *f != 0.0,
            Value::Null => false,
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Bool(_) => "BOOL",
            Value::Int8(_) => "INT8",
            Value::Int16(_) => "INT16",
            Value::Int32(_) => "INT32",
            Value::Int64(_) => "INT64",
            Value::Uint8(_) => "UINT8",
            Value::Uint16(_) => "UINT16",
            Value::Uint32(_) => "UINT32",
            Value::Uint64(_) => "UINT64",
            Value::Float32(_) => "FLOAT32",
            Value::Float64(_) => "FLOAT64",
            Value::RefMut(..) => "REF_MUT",
            Value::RefImm(..) => "REF_IMM",
            Value::Vec(_) => "VEC",
            Value::Struct(_) => "STRUCT",
            Value::Stack(_) => "STACK",
            Value::Queue(_) => "QUEUE",
            Value::HeapMin(_) => "HEAP_MIN",
            Value::HeapMax(_) => "HEAP_MAX",
            Value::Sparse(_) => "SPARSE",
            Value::BTree(_) => "BTREE",
            Value::Slice(_) => "SLICE",
            Value::Deque(_) => "DEQUE",
            Value::Bitset(_) => "BITSET",
            Value::Bitvec(_) => "BITVEC",
            Value::Option(_) => "OPTION",
            Value::Result(_) => "RESULT",
            Value::Future(_) => "FUTURE",
        }
    }

    /// Display a value for PRINT syscall.
    pub fn display(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int8(n) => n.to_string(),
            Value::Int16(n) => n.to_string(),
            Value::Int32(n) => n.to_string(),
            Value::Int64(n) => n.to_string(),
            Value::Uint8(n) => n.to_string(),
            Value::Uint16(n) => n.to_string(),
            Value::Uint32(n) => n.to_string(),
            Value::Uint64(n) => n.to_string(),
            Value::Float32(f) => f.to_string(),
            Value::Float64(f) => f.to_string(),
            Value::Vec(v) => {
                // Try to print as ASCII string if all elements are Uint8
                let chars: Option<Vec<u8>> =
                    v.0.iter()
                        .map(|x| match x {
                            Value::Uint8(c) => Some(*c),
                            _ => None,
                        })
                        .collect();
                match chars {
                    Some(bytes) => {
                        String::from_utf8(bytes).unwrap_or_else(|_| format!("{:?}", v.0))
                    }
                    None => format!(
                        "[{}]",
                        v.0.iter()
                            .map(|x| x.display())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            }
            Value::Struct(s) => format!("STRUCT({} fields)", s.0.len()),
            Value::Stack(s) => format!("STACK({} items)", s.0.len()),
            Value::Queue(q) => format!("QUEUE({} items)", q.0.len()),
            Value::HeapMin(h) => format!("HEAP_MIN({} items)", h.0.len()),
            Value::HeapMax(h) => format!("HEAP_MAX({} items)", h.0.len()),
            Value::Sparse(s) => format!("SPARSE({} entries)", s.len()),
            Value::BTree(b) => format!("BTREE({} entries)", b.len()),
            Value::Slice(s) => format!("SLICE({} items)", s.len()),
            Value::Deque(d) => format!("DEQUE({} items)", d.0.len()),
            Value::Bitset(b) => format!("BITSET({} bits, {} set)", b.len, b.popcount()),
            Value::Bitvec(bv) => format!("BITVEC({} bits)", bv.bit_len),
            Value::RefMut(g, i) => {
                format!("REF_MUT({}:{})", if *g { "global" } else { "local" }, i)
            }
            Value::RefImm(g, i) => {
                format!("REF_IMM({}:{})", if *g { "global" } else { "local" }, i)
            }
            Value::Option(o) => match o.as_ref() {
                FasmOption::None => "None".into(),
                FasmOption::Some(v) => format!("Some({})", v.display()),
            },
            Value::Result(r) => match r.as_ref() {
                FasmResult::Ok(v) => format!("Ok({})", v.display()),
                FasmResult::Err(c) => format!("Err(0x{:02X})", c),
            },
            Value::Future(Some(v)) => format!("Future(resolved: {})", v.display()),
            Value::Future(None) => "Future(pending)".into(),
        }
    }
}

// Arithmetic helpers — convert to f64 for generic ops then cast back
impl Value {
    pub fn add(&self, other: &Value) -> Option<Value> {
        numeric_op(self, other, |a, b| a + b, |a, b| a + b)
    }
    pub fn sub(&self, other: &Value) -> Option<Value> {
        numeric_op(self, other, |a, b| a - b, |a, b| a - b)
    }
    pub fn mul(&self, other: &Value) -> Option<Value> {
        numeric_op(self, other, |a, b| a * b, |a, b| a * b)
    }
    pub fn div(&self, other: &Value) -> Option<Value> {
        numeric_op(self, other, |a, b| a / b, |a, b| a / b)
    }
    pub fn rem(&self, other: &Value) -> Option<Value> {
        numeric_op(self, other, |a, b| a % b, |a, b| a % b)
    }

    pub fn neg(&self) -> Option<Value> {
        Some(match self {
            Value::Int8(n) => Value::Int8(-n),
            Value::Int16(n) => Value::Int16(-n),
            Value::Int32(n) => Value::Int32(-n),
            Value::Int64(n) => Value::Int64(-n),
            Value::Float32(f) => Value::Float32(-f),
            Value::Float64(f) => Value::Float64(-f),
            _ => return None,
        })
    }

    pub fn cmp_lt(&self, other: &Value) -> Option<bool> {
        numeric_cmp(self, other, |a, b| a < b, |a, b| a < b)
    }
    pub fn cmp_lte(&self, other: &Value) -> Option<bool> {
        numeric_cmp(self, other, |a, b| a <= b, |a, b| a <= b)
    }
    pub fn cmp_gt(&self, other: &Value) -> Option<bool> {
        numeric_cmp(self, other, |a, b| a > b, |a, b| a > b)
    }
    pub fn cmp_gte(&self, other: &Value) -> Option<bool> {
        numeric_cmp(self, other, |a, b| a >= b, |a, b| a >= b)
    }

    pub fn eq_val(&self, other: &Value) -> bool {
        // For numeric types, coerce both to i64 for comparison so that
        // e.g. Uint32(0) == Int32(0). This avoids type-tag mismatches
        // when comparing LEN output (UINT32) with integer literals (INT32).
        let ai = numeric_as_i64(self);
        let bi = numeric_as_i64(other);
        match (ai, bi) {
            (Some(a), Some(b)) => a == b,
            // Float comparisons
            _ => {
                let af = numeric_as_f64(self);
                let bf = numeric_as_f64(other);
                match (af, bf) {
                    (Some(a), Some(b)) => a == b,
                    _ => self == other, // exact equality for non-numeric types
                }
            }
        }
    }

    // Bitwise — integer only
    pub fn bit_and(&self, other: &Value) -> Option<Value> {
        bitwise_op(self, other, |a, b| a & b)
    }
    pub fn bit_or(&self, other: &Value) -> Option<Value> {
        bitwise_op(self, other, |a, b| a | b)
    }
    pub fn bit_xor(&self, other: &Value) -> Option<Value> {
        bitwise_op(self, other, |a, b| a ^ b)
    }
    pub fn bit_not(&self) -> Option<Value> {
        Some(match self {
            Value::Int8(n) => Value::Int8(!n),
            Value::Int16(n) => Value::Int16(!n),
            Value::Int32(n) => Value::Int32(!n),
            Value::Int64(n) => Value::Int64(!n),
            Value::Uint8(n) => Value::Uint8(!n),
            Value::Uint16(n) => Value::Uint16(!n),
            Value::Uint32(n) => Value::Uint32(!n),
            Value::Uint64(n) => Value::Uint64(!n),
            _ => return None,
        })
    }
    pub fn shl(&self, shift: u32) -> Option<Value> {
        Some(match self {
            Value::Int32(n) => Value::Int32(n << shift),
            Value::Int64(n) => Value::Int64(n << shift),
            Value::Uint32(n) => Value::Uint32(n << shift),
            Value::Uint64(n) => Value::Uint64(n << shift),
            _ => return None,
        })
    }
    pub fn shr(&self, shift: u32) -> Option<Value> {
        Some(match self {
            Value::Int32(n) => Value::Int32(n >> shift),
            Value::Int64(n) => Value::Int64(n >> shift),
            Value::Uint32(n) => Value::Uint32(n >> shift),
            Value::Uint64(n) => Value::Uint64(n >> shift),
            _ => return None,
        })
    }
}

/// Extract an integer representation from any numeric Value. Returns None for non-numeric types.
pub(crate) fn numeric_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(*b as i64),
        Value::Int8(n) => Some(*n as i64),
        Value::Int16(n) => Some(*n as i64),
        Value::Int32(n) => Some(*n as i64),
        Value::Int64(n) => Some(*n),
        Value::Uint8(n) => Some(*n as i64),
        Value::Uint16(n) => Some(*n as i64),
        Value::Uint32(n) => Some(*n as i64),
        Value::Uint64(n) => Some(*n as i64),
        _ => None,
    }
}

/// Extract a u64 from any integer Value. Returns None for non-integer types.
pub(crate) fn numeric_as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Bool(b) => Some(*b as u64),
        Value::Int8(n) => Some(*n as u64),
        Value::Int16(n) => Some(*n as u64),
        Value::Int32(n) => Some(*n as u64),
        Value::Int64(n) => Some(*n as u64),
        Value::Uint8(n) => Some(*n as u64),
        Value::Uint16(n) => Some(*n as u64),
        Value::Uint32(n) => Some(*n as u64),
        Value::Uint64(n) => Some(*n),
        _ => None,
    }
}

/// Extract a float representation from any numeric Value (floats only — int callers use numeric_as_i64).
pub(crate) fn numeric_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Float32(f) => Some(*f as f64),
        Value::Float64(f) => Some(*f),
        _ => None,
    }
}

fn numeric_op(
    a: &Value,
    b: &Value,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> Option<Value> {
    match (a, b) {
        (Value::Int8(x), Value::Int8(y)) => Some(Value::Int8(int_op(*x as i64, *y as i64) as i8)),
        (Value::Int16(x), Value::Int16(y)) => {
            Some(Value::Int16(int_op(*x as i64, *y as i64) as i16))
        }
        (Value::Int32(x), Value::Int32(y)) => {
            Some(Value::Int32(int_op(*x as i64, *y as i64) as i32))
        }
        (Value::Int64(x), Value::Int64(y)) => Some(Value::Int64(int_op(*x, *y))),
        (Value::Uint8(x), Value::Uint8(y)) => {
            Some(Value::Uint8(int_op(*x as i64, *y as i64) as u8))
        }
        (Value::Uint16(x), Value::Uint16(y)) => {
            Some(Value::Uint16(int_op(*x as i64, *y as i64) as u16))
        }
        (Value::Uint32(x), Value::Uint32(y)) => {
            Some(Value::Uint32(int_op(*x as i64, *y as i64) as u32))
        }
        (Value::Uint64(x), Value::Uint64(y)) => {
            Some(Value::Uint64(int_op(*x as i64, *y as i64) as u64))
        }
        (Value::Float32(x), Value::Float32(y)) => {
            Some(Value::Float32(float_op(*x as f64, *y as f64) as f32))
        }
        (Value::Float64(x), Value::Float64(y)) => Some(Value::Float64(float_op(*x, *y))),
        _ => None,
    }
}

fn numeric_cmp(
    a: &Value,
    b: &Value,
    int_cmp: impl Fn(i64, i64) -> bool,
    float_cmp: impl Fn(f64, f64) -> bool,
) -> Option<bool> {
    // Try same-type comparison first (exact), then fall back to i64 coercion
    // so that e.g. UINT32 len compared with INT32 literal works.
    match (a, b) {
        (Value::Int8(x), Value::Int8(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Int16(x), Value::Int16(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Int32(x), Value::Int32(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Int64(x), Value::Int64(y)) => Some(int_cmp(*x, *y)),
        (Value::Uint8(x), Value::Uint8(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Uint16(x), Value::Uint16(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Uint32(x), Value::Uint32(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Uint64(x), Value::Uint64(y)) => Some(int_cmp(*x as i64, *y as i64)),
        (Value::Float32(x), Value::Float32(y)) => Some(float_cmp(*x as f64, *y as f64)),
        (Value::Float64(x), Value::Float64(y)) => Some(float_cmp(*x, *y)),
        // Cross-type numeric coercion via i64
        _ => {
            let ai = numeric_as_i64(a)?;
            let bi = numeric_as_i64(b)?;
            Some(int_cmp(ai, bi))
        }
    }
}

fn bitwise_op(a: &Value, b: &Value, op: impl Fn(u64, u64) -> u64) -> Option<Value> {
    match (a, b) {
        (Value::Int8(x), Value::Int8(y)) => Some(Value::Int8(op(*x as u64, *y as u64) as i8)),
        (Value::Int16(x), Value::Int16(y)) => Some(Value::Int16(op(*x as u64, *y as u64) as i16)),
        (Value::Int32(x), Value::Int32(y)) => Some(Value::Int32(op(*x as u64, *y as u64) as i32)),
        (Value::Int64(x), Value::Int64(y)) => Some(Value::Int64(op(*x as u64, *y as u64) as i64)),
        (Value::Uint8(x), Value::Uint8(y)) => Some(Value::Uint8(op(*x as u64, *y as u64) as u8)),
        (Value::Uint16(x), Value::Uint16(y)) => {
            Some(Value::Uint16(op(*x as u64, *y as u64) as u16))
        }
        (Value::Uint32(x), Value::Uint32(y)) => {
            Some(Value::Uint32(op(*x as u64, *y as u64) as u32))
        }
        (Value::Uint64(x), Value::Uint64(y)) => Some(Value::Uint64(op(*x, *y))),
        _ => None,
    }
}
