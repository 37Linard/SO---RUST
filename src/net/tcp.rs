//! TCP minimo (Fase 25): so' o suficiente pra um handshake real + eco de
//! dados + fechamento gracioso, sem retransmissao/congestionamento/janela
//! deslizante de verdade -- uma conexao por vez (mesmo espirito de "uma
//! coisa de cada vez" ja usado em outras partes do projeto, ex. terminal
//! unico, processo em foreground unico). Ao contrario do UDP, o checksum
//! de TCP e' obrigatorio (a maioria das pilhas reais rejeita segmento com
//! checksum errado) -- calculado com pseudo-cabecalho IPv4 (RFC 793).

use super::ipv4;
use alloc::vec::Vec;

pub const FLAG_FIN: u8 = 0x01;
pub const FLAG_SYN: u8 = 0x02;
#[allow(dead_code)]
pub const FLAG_RST: u8 = 0x04;
pub const FLAG_PSH: u8 = 0x08;
pub const FLAG_ACK: u8 = 0x10;

pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
}

impl TcpHeader {
    /// Retorna o cabecalho parseado + o offset (em bytes) onde os dados
    /// comecam -- pode ser maior que 20 se o remetente mandou opcoes
    /// (MSS, window scale, etc), que a gente so' pula sem interpretar.
    pub fn parse(bytes: &[u8]) -> Option<(TcpHeader, usize)> {
        if bytes.len() < 20 {
            return None;
        }
        let data_offset = ((bytes[12] >> 4) as usize) * 4;
        if data_offset < 20 || data_offset > bytes.len() {
            return None;
        }
        Some((
            TcpHeader {
                src_port: u16::from_be_bytes([bytes[0], bytes[1]]),
                dst_port: u16::from_be_bytes([bytes[2], bytes[3]]),
                seq: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                ack: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
                flags: bytes[13],
                window: u16::from_be_bytes([bytes[14], bytes[15]]),
            },
            data_offset,
        ))
    }

    /// Monta o segmento (cabecalho de 20 bytes, sem opcoes, + payload) e
    /// calcula o checksum com pseudo-cabecalho IPv4 (obrigatorio em TCP,
    /// diferente do UDP -- reaproveita `ipv4::checksum`, so' monta o
    /// buffer de entrada diferente).
    pub fn write(&self, src_ip: [u8; 4], dst_ip: [u8; 4], payload: &[u8], out: &mut Vec<u8>) {
        let mut segment = Vec::with_capacity(20 + payload.len());
        segment.extend_from_slice(&self.src_port.to_be_bytes());
        segment.extend_from_slice(&self.dst_port.to_be_bytes());
        segment.extend_from_slice(&self.seq.to_be_bytes());
        segment.extend_from_slice(&self.ack.to_be_bytes());
        segment.push(5 << 4); // data offset = 5 palavras (20 bytes), sem opcoes
        segment.push(self.flags);
        segment.extend_from_slice(&self.window.to_be_bytes());
        segment.extend_from_slice(&0u16.to_be_bytes()); // checksum (calculado abaixo)
        segment.extend_from_slice(&0u16.to_be_bytes()); // urgent pointer
        segment.extend_from_slice(payload);

        let mut pseudo = Vec::with_capacity(12 + segment.len());
        pseudo.extend_from_slice(&src_ip);
        pseudo.extend_from_slice(&dst_ip);
        pseudo.push(0);
        pseudo.push(ipv4::PROTO_TCP);
        pseudo.extend_from_slice(&(segment.len() as u16).to_be_bytes());
        pseudo.extend_from_slice(&segment);

        let cksum = ipv4::checksum(&pseudo);
        segment[16..18].copy_from_slice(&cksum.to_be_bytes());

        out.extend_from_slice(&segment);
    }
}
