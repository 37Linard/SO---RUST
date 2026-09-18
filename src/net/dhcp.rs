//! Cliente DHCP (RFC 2131) minimo: DISCOVER -> OFFER -> REQUEST -> ACK.
//! Sem renovacao de lease/timeout/retransmissao -- pede um IP uma vez
//! (comando `dhcp` no shell) e usa ele ate o proximo boot. Mesma filosofia
//! de "prova de conceito, nao stack completa" do TCP em `mod.rs`.

use super::{send_ipv4_packet_from, set_our_ip, udp, BROADCAST_IP, BROADCAST_MAC};
use crate::net::ipv4;
use crate::rtl8139;
use alloc::vec::Vec;
use core::arch::x86_64::_rdtsc;
use spin::Mutex;

pub const SERVER_PORT: u16 = 67;
pub const CLIENT_PORT: u16 = 68;

const OP_REQUEST: u8 = 1;
const OP_REPLY: u8 = 2;
const HTYPE_ETHERNET: u8 = 1;
const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const FLAG_BROADCAST: u16 = 0x8000;

const OPT_MESSAGE_TYPE: u8 = 53;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_SERVER_ID: u8 = 54;
const OPT_END: u8 = 255;

const MSG_DISCOVER: u8 = 1;
const MSG_OFFER: u8 = 2;
const MSG_REQUEST: u8 = 3;
const MSG_ACK: u8 = 5;
const MSG_NAK: u8 = 6;

#[derive(Clone, Copy)]
enum State {
    Idle,
    Discovering { xid: u32 },
    Requesting { xid: u32, offered_ip: [u8; 4] },
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

fn new_xid() -> u32 {
    (unsafe { _rdtsc() }) as u32
}

/// Monta um pacote BOOTP/DHCP (sem opcoes alem das passadas) com nosso MAC
/// no `chaddr`, sempre com a flag de broadcast ligada (RFC 2131 4.1) --
/// mais simples que decodificar unicast pra um host sem IP ainda.
fn build_packet(mac: [u8; 6], xid: u32, msg_type: u8, extra: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::with_capacity(300);
    out.push(OP_REQUEST);
    out.push(HTYPE_ETHERNET);
    out.push(6); // hlen
    out.push(0); // hops
    out.extend_from_slice(&xid.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // secs
    out.extend_from_slice(&FLAG_BROADCAST.to_be_bytes());
    out.extend_from_slice(&[0; 4]); // ciaddr
    out.extend_from_slice(&[0; 4]); // yiaddr
    out.extend_from_slice(&[0; 4]); // siaddr
    out.extend_from_slice(&[0; 4]); // giaddr
    out.extend_from_slice(&mac);
    out.extend_from_slice(&[0; 10]); // resto do chaddr (16 bytes total)
    out.extend_from_slice(&[0; 64]); // sname
    out.extend_from_slice(&[0; 128]); // file
    out.extend_from_slice(&MAGIC_COOKIE);
    out.push(OPT_MESSAGE_TYPE);
    out.push(1);
    out.push(msg_type);
    for (code, value) in extra {
        out.push(*code);
        out.push(value.len() as u8);
        out.extend_from_slice(value);
    }
    out.push(OPT_END);
    out
}

fn send_packet(bytes: &[u8]) {
    let datagram = udp::UdpDatagram {
        src_port: CLIENT_PORT,
        dst_port: SERVER_PORT,
        payload: bytes,
    };
    let mut udp_out = Vec::new();
    datagram.write(&mut udp_out);
    send_ipv4_packet_from([0, 0, 0, 0], BROADCAST_MAC, BROADCAST_IP, ipv4::PROTO_UDP, &udp_out);
}

/// Inicia o processo (comando `dhcp` no shell): manda DISCOVER em
/// broadcast. Resposta (OFFER/ACK) chega assincronamente via
/// `handle_packet`, chamado pela `NetPollTask` ja rodando (comando `net`).
pub fn start() -> Result<(), &'static str> {
    let mac = rtl8139::mac_address().ok_or("rede nao inicializada -- rode `net` primeiro")?;
    let xid = new_xid();
    *STATE.lock() = State::Discovering { xid };
    send_packet(&build_packet(mac, xid, MSG_DISCOVER, &[]));
    Ok(())
}

fn option(options: &[u8], code: u8) -> Option<&[u8]> {
    let mut i = 0;
    while i + 1 < options.len() {
        let opt = options[i];
        if opt == OPT_END {
            break;
        }
        let len = options[i + 1] as usize;
        let start = i + 2;
        if start + len > options.len() {
            break;
        }
        if opt == code {
            return Some(&options[start..start + len]);
        }
        i = start + len;
    }
    None
}

/// Processa uma resposta do servidor (OFFER ou ACK/NAK), chamada pelo
/// `handle_udp` quando um datagrama chega na porta 68.
pub fn handle_packet(body: &[u8]) {
    if body.len() < 240 || body[0] != OP_REPLY || body[236..240] != MAGIC_COOKIE {
        return;
    }
    let xid = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let mut yiaddr = [0u8; 4];
    yiaddr.copy_from_slice(&body[16..20]);
    let options = &body[240..];
    let Some(msg_type) = option(options, OPT_MESSAGE_TYPE).and_then(|v| v.first().copied()) else {
        return;
    };

    let mut state = STATE.lock();
    match (*state, msg_type) {
        (State::Discovering { xid: our_xid }, MSG_OFFER) if our_xid == xid => {
            let Some(server_id) = option(options, OPT_SERVER_ID) else {
                return;
            };
            if server_id.len() != 4 {
                return;
            }
            let mut server = [0u8; 4];
            server.copy_from_slice(server_id);
            let Some(mac) = rtl8139::mac_address() else {
                return;
            };
            let extra: [(u8, &[u8]); 2] = [(OPT_REQUESTED_IP, &yiaddr), (OPT_SERVER_ID, &server)];
            send_packet(&build_packet(mac, xid, MSG_REQUEST, &extra));
            *state = State::Requesting { xid, offered_ip: yiaddr };
        }
        (State::Requesting { xid: our_xid, offered_ip, .. }, MSG_ACK) if our_xid == xid => {
            set_our_ip(offered_ip);
            *state = State::Idle;
            drop(state);
            crate::println!(
                "dhcp: IP {}.{}.{}.{} obtido",
                offered_ip[0], offered_ip[1], offered_ip[2], offered_ip[3]
            );
            crate::serial_println!(
                "dhcp: IP {}.{}.{}.{} obtido",
                offered_ip[0], offered_ip[1], offered_ip[2], offered_ip[3]
            );
        }
        (State::Requesting { xid: our_xid, .. }, MSG_NAK) if our_xid == xid => {
            *state = State::Idle;
            drop(state);
            crate::println!("dhcp: pedido recusado (NAK) pelo servidor");
            crate::serial_println!("dhcp: pedido recusado (NAK) pelo servidor");
        }
        _ => {}
    }
}
