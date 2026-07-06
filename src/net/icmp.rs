//! ICMP echo request/reply (ping) -- cabecalho de 8 bytes + payload.

use super::ipv4::checksum;
use alloc::vec::Vec;

pub const TYPE_ECHO_REQUEST: u8 = 8;
pub const TYPE_ECHO_REPLY: u8 = 0;

pub struct IcmpEcho<'a> {
    pub identifier: u16,
    pub sequence: u16,
    pub payload: &'a [u8],
}

impl<'a> IcmpEcho<'a> {
    /// So' reconhece echo request (tipo 8) -- e' tudo que o `ping` manda.
    pub fn parse_request(bytes: &'a [u8]) -> Option<IcmpEcho<'a>> {
        if bytes.len() < 8 || bytes[0] != TYPE_ECHO_REQUEST {
            return None;
        }
        let identifier = u16::from_be_bytes([bytes[4], bytes[5]]);
        let sequence = u16::from_be_bytes([bytes[6], bytes[7]]);
        Some(IcmpEcho {
            identifier,
            sequence,
            payload: &bytes[8..],
        })
    }

    /// Monta uma resposta (echo reply) com o mesmo identifier/sequence/
    /// payload do request -- e' isso que faz o `ping` do lado de fora
    /// reconhecer a resposta como valida.
    pub fn write_reply(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.push(TYPE_ECHO_REPLY);
        out.push(0); // code
        out.extend_from_slice(&0u16.to_be_bytes()); // checksum (calculado abaixo)
        out.extend_from_slice(&self.identifier.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(self.payload);

        let cksum = checksum(&out[start..]);
        out[start + 2..start + 4].copy_from_slice(&cksum.to_be_bytes());
    }
}
