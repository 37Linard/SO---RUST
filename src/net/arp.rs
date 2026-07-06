//! ARP (request/reply) pra IPv4 sobre Ethernet -- 28 bytes, sem opcoes.

use alloc::vec::Vec;

pub const OPER_REQUEST: u16 = 1;
pub const OPER_REPLY: u16 = 2;

pub struct ArpPacket {
    pub oper: u16,
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_mac: [u8; 6],
    pub target_ip: [u8; 4],
}

impl ArpPacket {
    pub fn parse(bytes: &[u8]) -> Option<ArpPacket> {
        if bytes.len() < 28 {
            return None;
        }
        let htype = u16::from_be_bytes([bytes[0], bytes[1]]);
        let ptype = u16::from_be_bytes([bytes[2], bytes[3]]);
        if htype != 1 || ptype != 0x0800 || bytes[4] != 6 || bytes[5] != 4 {
            return None; // so' suporta Ethernet/IPv4, mesmo caso de uso do ping
        }
        let oper = u16::from_be_bytes([bytes[6], bytes[7]]);
        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&bytes[8..14]);
        let mut sender_ip = [0u8; 4];
        sender_ip.copy_from_slice(&bytes[14..18]);
        let mut target_mac = [0u8; 6];
        target_mac.copy_from_slice(&bytes[18..24]);
        let mut target_ip = [0u8; 4];
        target_ip.copy_from_slice(&bytes[24..28]);
        Some(ArpPacket {
            oper,
            sender_mac,
            sender_ip,
            target_mac,
            target_ip,
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&1u16.to_be_bytes()); // htype = Ethernet
        out.extend_from_slice(&0x0800u16.to_be_bytes()); // ptype = IPv4
        out.push(6); // hlen
        out.push(4); // plen
        out.extend_from_slice(&self.oper.to_be_bytes());
        out.extend_from_slice(&self.sender_mac);
        out.extend_from_slice(&self.sender_ip);
        out.extend_from_slice(&self.target_mac);
        out.extend_from_slice(&self.target_ip);
    }
}
