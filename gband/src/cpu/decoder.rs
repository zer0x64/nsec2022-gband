use num_enum::TryFromPrimitive;

#[derive(TryFromPrimitive, Clone, Copy)]
#[repr(u8)]
pub enum Register {
    B = 0,
    C = 1,
    D = 2,
    E = 3,
    H = 4,
    L = 5,
    // HL = 6, // Used in instruction encoding for (HL) and immediates, not truly used as a register
    A = 7,
}

#[derive(TryFromPrimitive, Clone, Copy)]
#[repr(u8)]
pub enum RegisterPair {
    BC = 0,
    DE = 1,
    HL = 2,
    SP = 3,
    AF = 4, // Only used in Push and Pop, otherwise SP is used. Can't use the same int in rust
}

#[derive(TryFromPrimitive, Clone, Copy)]
#[repr(u8)]
pub enum Alu {
    Add = 0,
    Adc = 1,
    Sub = 2,
    Sbc = 3,
    And = 4,
    Xor = 5,
    Or = 6,
    Cp = 7,
}

#[derive(TryFromPrimitive, Clone, Copy)]
#[repr(u8)]
enum Rot {
    Rlc = 0,
    Rrc = 1,
    Rl = 2,
    Rr = 3,
    Sla = 4,
    Sra = 5,
    Swap = 6,
    Srl = 7
}

#[derive(Clone, Copy)]
pub enum OpMemAddress16 {
    Register(RegisterPair),
    RegisterIncrease(RegisterPair),
    RegisterDecrease(RegisterPair),
    Immediate,
}

#[derive(Clone, Copy)]
pub enum OpMemAddress8 {
    Register(Register),
    Immediate,
}

#[derive(Clone, Copy)]
pub enum Opcode {
    Unknown,

    // 8 bits load
    LdRR(Register, Register),
    LdRImm(Register),
    LdRMem(Register, OpMemAddress16),
    LdMemR(OpMemAddress16, Register),
    LdMemImm(RegisterPair),
    LdhRead(Register, OpMemAddress8),
    LdhWrite(OpMemAddress8, Register),

    // 16 bits load
    Ld16RImm(RegisterPair),
    Ld16MemSp,
    Ld16SpHL,
    Push(RegisterPair),
    Pop(RegisterPair),

    // 8 bits ALU
    AluR(Alu, Register),
    AluImm(Alu),
    AluMem(Alu),
    IncR(Register),
    IncMem,
    DecR(Register),
    DecMem,
    Daa,
    Cpl,

    // 16 bits ALU
    Add16HL(RegisterPair),
    Add16SPSigned,
    Inc16R(RegisterPair),
    Dec16R(RegisterPair),
    Ld16HLSPSigned
}

impl From<u8> for Opcode {
    fn from(op: u8) -> Self {
        // Typical binary encodings are xx,yyy,zzz and xx,ppq,zzz
        match &op {
            0x40..=0x45 | 0x47..=0x4D | 0x4F..=0x55 |
            0x57..=0x5D | 0x5F..=0x65 | 0x67..=0x6D |
            0x6F..=0x6F | 0x78..=0x7D | 0x7F => {
                // Encoding: 01,yyy,zzz y: target reg8 z: source reg8
                let target = Register::try_from((op & 0o070) >> 3).expect("LD r,r: Unexpected target register");
                let source = Register::try_from(op & 0o007).expect("LD r,r: Unexpected source register");
                Self::LdRR(target, source)
            },
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E => {
                // Encoding: 00,yyy,110 y: target reg8
                let target = Register::try_from((op & 0o070) >> 3).expect("LD r,n: Unexpected target register");
                Self::LdRImm(target)
            },
            0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x76 | 0x7E => {
                // Encoding: 01,yyy,110 y: target reg8
                let target = Register::try_from((op & 0o070) >> 3).expect("LD r,(HL): Unexpected target register");
                Self::LdRMem(target, OpMemAddress16::Register(RegisterPair::HL))
            },
            0x0A | 0x1A => {
                // Encoding: 00,pp1,010 p: source reg16 (BC and DE only)
                let source = RegisterPair::try_from((op & 0b00110000) >> 4).expect("LD A,(rr): Unexpected source register");
                Self::LdRMem(Register::A, OpMemAddress16::Register(source))
            },
            0x2A => {
                // Encoding: 00,101,010
                Self::LdRMem(Register::A, OpMemAddress16::RegisterIncrease(RegisterPair::HL))
            },
            0x3A => {
                // Encoding: 00,111,010
                Self::LdRMem(Register::A, OpMemAddress16::RegisterDecrease(RegisterPair::HL))
            },
            0xFA => {
                // Encoding: 11,111,010
                Self::LdRMem(Register::A, OpMemAddress16::Immediate)
            },
            0x70..=0x77 => {
                // Encoding: 01,110,zzz z: source reg8
                let source = Register::try_from(op & 0o007).expect("LD (HL),r: Unexpected source register");
                Self::LdMemR(OpMemAddress16::Register(RegisterPair::HL), source)
            },
            0x02 | 0x12 => {
                // Encoding: 00,pp0,010 p: target reg16 (BC and DE only)
                let target = RegisterPair::try_from((op & 0b00110000) >> 4).expect("LD (rr),A: Unexpected target register");
                Self::LdMemR(OpMemAddress16::Register(target), Register::A)
            },
            0x22 => {
                // Encoding: 00,100,010
                Self::LdMemR(OpMemAddress16::RegisterIncrease(RegisterPair::HL), Register::A)
            },
            0x32 => {
                // Encoding: 00,110,010
                Self::LdMemR(OpMemAddress16::RegisterDecrease(RegisterPair::HL), Register::A)
            },
            0xEA => {
                // Encoding: 11_101_010
                Self::LdMemR(OpMemAddress16::Immediate, Register::A)
            }
            0x36 => {
                // Encoding: 00,110,110
                Self::LdMemImm(RegisterPair::HL)
            },
            0xF2 => {
                // Encoding: 11,110,010
                Self::LdhRead(Register::A, OpMemAddress8::Register(Register::C))
            },
            0xF0 => {
                // Encoding: 11,110,000
                Self::LdhRead(Register::A, OpMemAddress8::Immediate)
            },
            0xE2 => {
                // Encoding: 11,100,010
                Self::LdhWrite(OpMemAddress8::Register(Register::C), Register::A)
            },
            0xE0 => {
                // Encoding: 11,100,000
                Self::LdhWrite(OpMemAddress8::Immediate, Register::A)
            },
            0x01 | 0x11 | 0x21 | 0x31 => {
                // Encoding: 00,pp0,001 p: target reg16
                let target = RegisterPair::try_from((op & 0b00110000) >> 4).expect("LD rr,nn: Unexpected target register");
                Self::Ld16RImm(target)
            },
            0x08 => {
                // Encoding: 00,001,000
                Self::Ld16MemSp
            }
            0xF9 => {
                // Encoding: 11,111,001
                Self::Ld16SpHL
            },
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                // Encoding: 11,pp0,101 p: source reg16
                // This uses AF for 3, not SP
                let source = RegisterPair::try_from((op & 0b00110000) >> 4).expect("PUSH rr: Unexpected source register");
                Self::Push(if let RegisterPair::SP = source { RegisterPair::HL } else { source })
            },
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                // Encoding: 11,pp0,001 p: target reg16
                // This uses AF for 3, not SP
                let target = RegisterPair::try_from((op & 0b00110000) >> 4).expect("POP rr: Unexpected target register");
                Self::Pop(if let RegisterPair::SP = target { RegisterPair::HL } else { target })
            },
            0x80..=0x85 | 0x87..=0x8D | 0x8F..=0x95 |
            0x97..=0x9D | 0x9F..=0xA5 | 0xA7..=0xAD |
            0xAF..=0xB5 | 0xB7..=0xBD | 0xBF => {
                // Encoding: 10,yyy,zzz y: alu op z: source reg8
                let alu_op = Alu::try_from((op & 0o070) >> 3).expect("Alu r: Unexpected alu operation");
                let source = Register::try_from(op & 0o007).expect("Alu r: Unexpected source register");
                Self::AluR(alu_op, source)
            },
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                // Encoding: 11,yyy,110 y: alu op
                let alu_op = Alu::try_from((op & 0o070) >> 3).expect("Alu n: Unexpected alu operation");
                Self::AluImm(alu_op)
            },
            0x86 | 0x8E | 0x96 | 0x9E | 0xA6 | 0xAE | 0xB6 | 0xBE => {
                // Encoding: 10,yyy,110 y: alu op
                let alu_op = Alu::try_from((op & 0o070) >> 3).expect("Alu (HL): Unexpected alu operation");
                Self::AluMem(alu_op)
            },
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C => {
                // Encoding: 00,yyy,100 y: source reg8
                let source = Register::try_from((op & 0o070) >> 3).expect("INC r: Unexpected source register");
                Self::IncR(source)
            },
            0x34 => {
                // Encoding: 00,110,100
                Self::IncMem
            },
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D => {
                // Encoding: 00,yyy,101 y: source reg8
                let source = Register::try_from((op & 0o070) >> 3).expect("DEC r: Unexpected source register");
                Self::DecR(source)
            },
            0x35 => {
                // Encoding: 00,110,101
                Self::DecMem
            },
            0x27 => {
                // Encoding: 00,100,111
                Self::Daa
            },
            0x2F => {
                // Encoding: 00,101,111
                Self::Cpl
            },
            0x09 | 0x19 | 0x29 | 0x39 => {
                // Encoding: 00,pp1,001 p: source reg16
                let source = RegisterPair::try_from((op & 0b00110000) >> 4).expect("ADD HL,rr: Unexpected source register");
                Self::Add16HL(source)
            },
            0xE8 => {
                // Encoding: 11,101,000
                Self::Add16SPSigned
            },
            0x03 | 0x13 | 0x23 | 0x33 => {
                // Encoding: 00,pp0,011 p: source reg16
                let source = RegisterPair::try_from((op & 0b00110000) >> 4).expect("INC rr: Unexpected source register");
                Self::Inc16R(source)
            },
            0x0B | 0x1B | 0x2B | 0x3B => {
                // Encoding: 00,pp1,011 p: source reg16
                let source = RegisterPair::try_from((op & 0b00110000) >> 4).expect("DEC rr: Unexpected source register");
                Self::Dec16R(source)
            },
            0xF8 => {
                // Encoding: 11,111,000
                Self::Ld16HLSPSigned
            }
            _ => Self::Unknown
        }
    }
}

impl Opcode {
    pub fn cycles(&self) -> u8 {
        match self {
            Self::Unknown => 1,
            Self::LdRR(_, _) => 1,
            Self::LdRImm(_) => 2,
            Self::LdRMem(_, mem) => {
                match mem {
                    OpMemAddress16::Immediate => 4,
                    _ => 2
                }
            },
            Self::LdMemR(mem, _) => {
                match mem {
                    OpMemAddress16::Immediate => 4,
                    _ => 2
                }
            },
            Self::LdMemImm(_) => 3,
            Self::LdhRead(_, mem) => {
                match mem {
                    OpMemAddress8::Register(_) => 2,
                    OpMemAddress8::Immediate => 3
                }
            }
            Self::LdhWrite(mem, _) => {
                match mem {
                    OpMemAddress8::Register(_) => 2,
                    OpMemAddress8::Immediate => 3
                }
            }
            Self::Ld16RImm(_) => 3,
            Self::Ld16MemSp => 5,
            Self::Ld16SpHL => 2,
            Self::Push(_) => 4,
            Self::Pop(_) => 3,
            Self::AluR(_, _) => 1,
            Self::AluImm(_) => 2,
            Self::AluMem(_) => 2,
            Self::IncR(_) => 1,
            Self::IncMem => 3,
            Self::DecR(_) => 1,
            Self::DecMem => 3,
            Self::Daa => 1,
            Self::Cpl => 1,
            Self::Add16HL(_) => 2,
            Self::Add16SPSigned => 4,
            Self::Inc16R(_) => 2,
            Self::Dec16R(_) => 2,
            Self::Ld16HLSPSigned => 4,
        }
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn test_all_instructions_implemented() {
    for i in 0u8..=255u8 {
        let opcode = Opcode::from(i);
        match i {
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                assert!(matches!(opcode, Opcode::Unknown), "{:#04X} should be unknown", i);
            }
            _ => {
                assert!(!matches!(opcode, Opcode::Unknown), "{:#04X} shouldn't be unknown", i);
            }
        }
    }
}
