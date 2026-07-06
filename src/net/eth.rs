//! Cabecalho Ethernet (14 bytes) -- serializacao manual byte a byte, mesmo
//! estilo de `elf.rs`/`fs.rs` (sem `#[repr(C)]` cast direto no buffer).

use alloc::vec::Vec;

pub const HEADER_LEN: usize = 14;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

pub struct EthHeader {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ethertype: u16,
}

impl EthHeader {
    pub fn parse(bytes: &[u8]) -> Option<EthHeader> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let mut dst_mac = [0u8; 6];
        let mut src_mac = [0u8; 6];
        dst_mac.copy_from_slice(&bytes[0..6]);
        src_mac.copy_from_slice(&bytes[6..12]);
        let ethertype = u16::from_be_bytes([bytes[12], bytes[13]]);
        Some(EthHeader {
            dst_mac,
            src_mac,
            ethertype,
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.dst_mac);
        out.extend_from_slice(&self.src_mac);
        out.extend_from_slice(&self.ethertype.to_be_bytes());
    }
}
