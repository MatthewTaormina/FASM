/// FASM primitive and composite types as represented in bytecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FasmType {
    Bool = 0x01,
    Int8 = 0x02,
    Int16 = 0x03,
    Int32 = 0x04,
    Int64 = 0x05,
    Uint8 = 0x06,
    Uint16 = 0x07,
    Uint32 = 0x08,
    Uint64 = 0x09,
    Float32 = 0x0A,
    Float64 = 0x0B,
    RefMut = 0x10,
    RefImm = 0x11,
    Vec = 0x20,
    Struct = 0x21,
    Stack = 0x22,
    Queue = 0x23,
    HeapMin = 0x24,
    HeapMax = 0x25,
    // High-performance collections
    Sparse = 0x26, // FxHashMap<u32, Value> — O(1) integer-keyed sparse array
    BTree = 0x27,  // BTreeMap<u32, Value>  — O(log n) ordered integer-keyed map
    Slice = 0x28,  // read-only sub-range view of a VEC
    Deque = 0x29,  // VecDeque — double-ended queue (prepend + append)
    Bitset = 0x2A, // Vec<u8> bit-addressable boolean array
    Bitvec = 0x2B, // Vec<u8> arbitrary-width bit field storage
    // Wrappers
    Option = 0x30,
    Result = 0x31,
    Future = 0x32,
    Null = 0xFF,
}

impl TryFrom<u8> for FasmType {
    type Error = String;
    fn try_from(b: u8) -> Result<Self, Self::Error> {
        match b {
            0x01 => Ok(FasmType::Bool),
            0x02 => Ok(FasmType::Int8),
            0x03 => Ok(FasmType::Int16),
            0x04 => Ok(FasmType::Int32),
            0x05 => Ok(FasmType::Int64),
            0x06 => Ok(FasmType::Uint8),
            0x07 => Ok(FasmType::Uint16),
            0x08 => Ok(FasmType::Uint32),
            0x09 => Ok(FasmType::Uint64),
            0x0A => Ok(FasmType::Float32),
            0x0B => Ok(FasmType::Float64),
            0x10 => Ok(FasmType::RefMut),
            0x11 => Ok(FasmType::RefImm),
            0x20 => Ok(FasmType::Vec),
            0x21 => Ok(FasmType::Struct),
            0x22 => Ok(FasmType::Stack),
            0x23 => Ok(FasmType::Queue),
            0x24 => Ok(FasmType::HeapMin),
            0x25 => Ok(FasmType::HeapMax),
            0x26 => Ok(FasmType::Sparse),
            0x27 => Ok(FasmType::BTree),
            0x28 => Ok(FasmType::Slice),
            0x29 => Ok(FasmType::Deque),
            0x2A => Ok(FasmType::Bitset),
            0x2B => Ok(FasmType::Bitvec),
            0x30 => Ok(FasmType::Option),
            0x31 => Ok(FasmType::Result),
            0x32 => Ok(FasmType::Future),
            0xFF => Ok(FasmType::Null),
            _ => Err(format!("Unknown type tag: 0x{:02X}", b)),
        }
    }
}
