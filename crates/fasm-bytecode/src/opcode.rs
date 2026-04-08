/// All FASM VM opcodes. Encoded as a single u8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    Nop = 0x00,
    // Memory
    Reserve = 0x01,
    Release = 0x02,
    // Data movement
    Mov = 0x03,
    Store = 0x04,
    Addr = 0x05,
    // Arithmetic
    Add = 0x10,
    Sub = 0x11,
    Mul = 0x12,
    Div = 0x13,
    Mod = 0x14,
    Neg = 0x15,
    // Comparison
    Eq = 0x20,
    Neq = 0x21,
    Lt = 0x22,
    Lte = 0x23,
    Gt = 0x24,
    Gte = 0x25,
    // Bitwise
    And = 0x30,
    Or = 0x31,
    Xor = 0x32,
    Not = 0x33,
    Shl = 0x34,
    Shr = 0x35,
    // Control flow
    Jmp = 0x40,
    Jz = 0x41,
    Jnz = 0x42,
    Call = 0x43,
    AsyncCall = 0x44,
    Ret = 0x45,
    Await = 0x46,
    TailCall = 0x47,
    TmpBlock = 0x48,
    EndTmp = 0x49,
    // Syscalls
    Syscall = 0x50,
    AsyncSyscall = 0x51,
    // Core collections (0x60–0x6F)
    Push = 0x60,
    Pop = 0x61,
    Enqueue = 0x62,
    Dequeue = 0x63,
    Peek = 0x64,
    GetIdx = 0x65,
    SetIdx = 0x66,
    GetField = 0x67,
    SetField = 0x68,
    HasField = 0x69,
    DelField = 0x6A,
    Len = 0x6B,
    // DEQUE extensions (0x6C–0x6E)
    Prepend = 0x6C,  // PUSH_FRONT: PREPEND deque, val
    PopBack = 0x6D,  // POP_BACK:   POP_BACK deque, target
    PeekBack = 0x6E, // PEEK_BACK:  PEEK_BACK deque, target
    // Wrapper types (0x70–0x77)
    Some_ = 0x70,
    IsSome = 0x71,
    Unwrap = 0x72,
    Ok_ = 0x73,
    Err_ = 0x74,
    IsOk = 0x75,
    UnwrapOk = 0x76,
    UnwrapErr = 0x77,
    // Type conversion
    Cast = 0x80,
    // Error handling
    Try = 0x90,
    Catch = 0x91,
    EndTry = 0x92,
    // VEC native operations (0xA0–0xA3)
    VecSort = 0xA0,   // VEC_SORT vec               — in-place unstable numeric sort
    VecFilter = 0xA1, // VEC_FILTER vec, op, val, tgt — op: 0=LT,1=EQ,2=GT
    VecMerge = 0xA2,  // VEC_MERGE va, vb, tgt       — merge two sorted VECs
    VecSlice = 0xA3,  // VEC_SLICE vec, start, len, tgt — copy sub-range → SLICE
    // SPARSE operations (0xA4–0xA7)
    SparseGet = 0xA4, // SPARSE_GET  sparse, key, tgt
    SparseSet = 0xA5, // SPARSE_SET  sparse, key, val
    SparseDel = 0xA6, // SPARSE_DEL  sparse, key
    SparseHas = 0xA7, // SPARSE_HAS  sparse, key, tgt_bool
    // BTREE operations (0xA8–0xAD)
    BTreeGet = 0xA8, // BTREE_GET   btree, key, tgt
    BTreeSet = 0xA9, // BTREE_SET   btree, key, val
    BTreeDel = 0xAA, // BTREE_DEL   btree, key
    BTreeHas = 0xAB, // BTREE_HAS   btree, key, tgt_bool
    BTreeMin = 0xAC, // BTREE_MIN   btree, tgt_key
    BTreeMax = 0xAD, // BTREE_MAX   btree, tgt_key
    // BITSET operations (0xB0–0xB8)
    BitSet_ = 0xB0,  // BIT_SET   bitset, idx       — set bit
    BitClr = 0xB1,   // BIT_CLR   bitset, idx       — clear bit
    BitGet = 0xB2,   // BIT_GET   bitset, idx, tgt  — get bit as BOOL
    BitFlip = 0xB3,  // BIT_FLIP  bitset, idx       — toggle bit
    BitCount = 0xB4, // BIT_COUNT bitset, tgt        — popcount → UINT32
    BitAnd = 0xB5,   // BIT_AND   dst, src           — in-place AND
    BitOr = 0xB6,    // BIT_OR    dst, src           — in-place OR
    BitXor = 0xB7,   // BIT_XOR   dst, src           — in-place XOR
    BitGrow = 0xB8,  // BIT_GROW  bitset, n_bits     — extend by N bits (zero-fill)
    // BITVEC operations (0xC0–0xC2)
    BitvecRead = 0xC0, // BITVEC_READ  bv, bit_start, bit_len, tgt — read N bits → UINT64
    BitvecWrite = 0xC1, // BITVEC_WRITE bv, bit_start, bit_len, val — write N bits
    BitvecPush = 0xC2, // BITVEC_PUSH  bv, val, bit_len            — append N bits
    // Special
    Halt = 0xFF,
}

impl TryFrom<u8> for Opcode {
    type Error = String;

    fn try_from(b: u8) -> Result<Self, Self::Error> {
        match b {
            0x00 => Ok(Opcode::Nop),
            0x01 => Ok(Opcode::Reserve),
            0x02 => Ok(Opcode::Release),
            0x03 => Ok(Opcode::Mov),
            0x04 => Ok(Opcode::Store),
            0x05 => Ok(Opcode::Addr),
            0x10 => Ok(Opcode::Add),
            0x11 => Ok(Opcode::Sub),
            0x12 => Ok(Opcode::Mul),
            0x13 => Ok(Opcode::Div),
            0x14 => Ok(Opcode::Mod),
            0x15 => Ok(Opcode::Neg),
            0x20 => Ok(Opcode::Eq),
            0x21 => Ok(Opcode::Neq),
            0x22 => Ok(Opcode::Lt),
            0x23 => Ok(Opcode::Lte),
            0x24 => Ok(Opcode::Gt),
            0x25 => Ok(Opcode::Gte),
            0x30 => Ok(Opcode::And),
            0x31 => Ok(Opcode::Or),
            0x32 => Ok(Opcode::Xor),
            0x33 => Ok(Opcode::Not),
            0x34 => Ok(Opcode::Shl),
            0x35 => Ok(Opcode::Shr),
            0x40 => Ok(Opcode::Jmp),
            0x41 => Ok(Opcode::Jz),
            0x42 => Ok(Opcode::Jnz),
            0x43 => Ok(Opcode::Call),
            0x44 => Ok(Opcode::AsyncCall),
            0x45 => Ok(Opcode::Ret),
            0x46 => Ok(Opcode::Await),
            0x47 => Ok(Opcode::TailCall),
            0x48 => Ok(Opcode::TmpBlock),
            0x49 => Ok(Opcode::EndTmp),
            0x50 => Ok(Opcode::Syscall),
            0x51 => Ok(Opcode::AsyncSyscall),
            0x60 => Ok(Opcode::Push),
            0x61 => Ok(Opcode::Pop),
            0x62 => Ok(Opcode::Enqueue),
            0x63 => Ok(Opcode::Dequeue),
            0x64 => Ok(Opcode::Peek),
            0x65 => Ok(Opcode::GetIdx),
            0x66 => Ok(Opcode::SetIdx),
            0x67 => Ok(Opcode::GetField),
            0x68 => Ok(Opcode::SetField),
            0x69 => Ok(Opcode::HasField),
            0x6A => Ok(Opcode::DelField),
            0x6B => Ok(Opcode::Len),
            0x6C => Ok(Opcode::Prepend),
            0x6D => Ok(Opcode::PopBack),
            0x6E => Ok(Opcode::PeekBack),
            0x70 => Ok(Opcode::Some_),
            0x71 => Ok(Opcode::IsSome),
            0x72 => Ok(Opcode::Unwrap),
            0x73 => Ok(Opcode::Ok_),
            0x74 => Ok(Opcode::Err_),
            0x75 => Ok(Opcode::IsOk),
            0x76 => Ok(Opcode::UnwrapOk),
            0x77 => Ok(Opcode::UnwrapErr),
            0x80 => Ok(Opcode::Cast),
            0x90 => Ok(Opcode::Try),
            0x91 => Ok(Opcode::Catch),
            0x92 => Ok(Opcode::EndTry),
            0xA0 => Ok(Opcode::VecSort),
            0xA1 => Ok(Opcode::VecFilter),
            0xA2 => Ok(Opcode::VecMerge),
            0xA3 => Ok(Opcode::VecSlice),
            0xA4 => Ok(Opcode::SparseGet),
            0xA5 => Ok(Opcode::SparseSet),
            0xA6 => Ok(Opcode::SparseDel),
            0xA7 => Ok(Opcode::SparseHas),
            0xA8 => Ok(Opcode::BTreeGet),
            0xA9 => Ok(Opcode::BTreeSet),
            0xAA => Ok(Opcode::BTreeDel),
            0xAB => Ok(Opcode::BTreeHas),
            0xAC => Ok(Opcode::BTreeMin),
            0xAD => Ok(Opcode::BTreeMax),
            0xB0 => Ok(Opcode::BitSet_),
            0xB1 => Ok(Opcode::BitClr),
            0xB2 => Ok(Opcode::BitGet),
            0xB3 => Ok(Opcode::BitFlip),
            0xB4 => Ok(Opcode::BitCount),
            0xB5 => Ok(Opcode::BitAnd),
            0xB6 => Ok(Opcode::BitOr),
            0xB7 => Ok(Opcode::BitXor),
            0xB8 => Ok(Opcode::BitGrow),
            0xC0 => Ok(Opcode::BitvecRead),
            0xC1 => Ok(Opcode::BitvecWrite),
            0xC2 => Ok(Opcode::BitvecPush),
            0xFF => Ok(Opcode::Halt),
            _ => Err(format!("Unknown opcode: 0x{:02X}", b)),
        }
    }
}
