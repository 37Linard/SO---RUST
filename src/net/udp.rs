//! Cabecalho UDP (8 bytes) -- sem estado/conexao, so' porta origem/destino
//! + tamanho. Checksum e' opcional em IPv4 (RFC 768) -- mandamos 0 (sem
//! checksum) pra nao precisar montar o pseudo-cabecalho IPv4+UDP, e nao
//! validamos o checksum de entrada (0 e' valor valido "sem checksum").

use alloc::vec::Vec;

pub const HEADER_LEN: usize = 8;

pub struct UdpDatagram<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpDatagram<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<UdpDatagram<'a>> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let src_port = u16::from_be_bytes([bytes[0], bytes[1]]);
        let dst_port = u16::from_be_bytes([bytes[2], bytes[3]]);
        let length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        if length < HEADER_LEN || length > bytes.len() {
            return None;
        }
        Some(UdpDatagram {
            src_port,
            dst_port,
            payload: &bytes[HEADER_LEN..length],
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        let length = (HEADER_LEN + self.payload.len()) as u16;
        out.extend_from_slice(&self.src_port.to_be_bytes());
        out.extend_from_slice(&self.dst_port.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // checksum = 0 (opcional, ver doc do modulo)
        out.extend_from_slice(self.payload);
    }
}
