//! Cabecalho IPv4 (20 bytes, sem opcoes) + checksum.

use alloc::vec::Vec;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub struct Ipv4Header {
    pub protocol: u8,
    pub src: [u8; 4],
    pub dst: [u8; 4],
    pub payload_len: usize,
}

/// Checksum de complemento de 1 (RFC 791) -- usado tanto pro cabecalho
/// IPv4 quanto (com pseudo-cabecalho, aqui nao precisamos pro ICMP puro)
/// outros protocolos. Soma todas as palavras de 16 bits, com carry-around.
pub fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32;
        i += 2;
    }
    if i < bytes.len() {
        sum += (bytes[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

impl Ipv4Header {
    pub fn parse(bytes: &[u8]) -> Option<(Ipv4Header, usize)> {
        if bytes.len() < 20 {
            return None;
        }
        let version = bytes[0] >> 4;
        let ihl = (bytes[0] & 0x0F) as usize * 4;
        if version != 4 || ihl < 20 || bytes.len() < ihl {
            return None;
        }
        let total_length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        let protocol = bytes[9];
        let mut src = [0u8; 4];
        src.copy_from_slice(&bytes[12..16]);
        let mut dst = [0u8; 4];
        dst.copy_from_slice(&bytes[16..20]);
        let payload_len = total_length.saturating_sub(ihl);
        Some((
            Ipv4Header {
                protocol,
                src,
                dst,
                payload_len,
            },
            ihl,
        ))
    }

    /// Monta o cabecalho (20 bytes, sem opcoes) + calcula o checksum.
    pub fn write(&self, out: &mut Vec<u8>) {
        let total_length = (20 + self.payload_len) as u16;
        let start = out.len();
        out.push(0x45); // version=4, IHL=5 (20 bytes)
        out.push(0); // DSCP/ECN
        out.extend_from_slice(&total_length.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // identification
        out.extend_from_slice(&0u16.to_be_bytes()); // flags/fragment offset
        out.push(64); // TTL
        out.push(self.protocol);
        out.extend_from_slice(&0u16.to_be_bytes()); // checksum (calculado abaixo)
        out.extend_from_slice(&self.src);
        out.extend_from_slice(&self.dst);

        let cksum = checksum(&out[start..start + 20]);
        out[start + 10..start + 12].copy_from_slice(&cksum.to_be_bytes());
    }
}
