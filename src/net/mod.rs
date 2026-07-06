pub mod arp;
pub mod eth;
pub mod icmp;
pub mod ipv4;
pub mod tcp;
pub mod udp;

use crate::rtl8139;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use spin::Mutex;

/// Endereco fixo do modo `user`/SLIRP do QEMU -- sem cliente DHCP (fora de
/// escopo, ver plano da Fase 23). So' funciona se o QEMU estiver usando a
/// rede padrao (10.0.2.0/24) -- documentado no README.
pub const OUR_IP: [u8; 4] = [10, 0, 2, 15];

/// Porta UDP do servidor de eco (Fase 24) -- qualquer datagrama recebido
/// aqui volta identico pro remetente, mesma ideia do responder de ICMP.
pub const UDP_ECHO_PORT: u16 = 7777;

/// Porta TCP do servidor de eco (Fase 25).
pub const TCP_ECHO_PORT: u16 = 7778;

#[derive(Clone, Copy, PartialEq)]
enum TcpState {
    SynReceived,
    Established,
    LastAck,
}

struct TcpConnection {
    state: TcpState,
    peer_mac: [u8; 6],
    peer_ip: [u8; 4],
    peer_port: u16,
    our_seq: u32,
    peer_seq: u32,
}

/// Uma conexao TCP por vez (mesmo espirito de "uma coisa de cada vez" ja
/// usado em outras partes do projeto -- terminal unico, processo em
/// foreground unico). Um SYN novo sempre substitui a conexao atual.
static TCP_CONNECTION: Mutex<Option<TcpConnection>> = Mutex::new(None);

/// Monta Ethernet + IPv4 em volta de um payload ja pronto (corpo de
/// ICMP/UDP/etc) e manda pro fio -- reaproveitado por `handle_ipv4`
/// (ICMP) e `handle_udp` pra nao duplicar o "framing" duas vezes.
fn send_ipv4_packet(dst_mac: [u8; 6], dst_ip: [u8; 4], protocol: u8, payload: &[u8]) {
    let Some(our_mac) = rtl8139::mac_address() else {
        return;
    };
    let mut out = Vec::new();
    eth::EthHeader {
        dst_mac,
        src_mac: our_mac,
        ethertype: eth::ETHERTYPE_IPV4,
    }
    .write(&mut out);
    ipv4::Ipv4Header {
        protocol,
        src: OUR_IP,
        dst: dst_ip,
        payload_len: payload.len(),
    }
    .write(&mut out);
    out.extend_from_slice(payload);
    rtl8139::send_packet(&out);
}

fn handle_frame(frame: &[u8]) {
    let Some(header) = eth::EthHeader::parse(frame) else {
        return;
    };
    let payload = &frame[eth::HEADER_LEN..];
    match header.ethertype {
        eth::ETHERTYPE_ARP => handle_arp(payload),
        eth::ETHERTYPE_IPV4 => handle_ipv4(&header, payload),
        _ => {}
    }
}

fn handle_arp(payload: &[u8]) {
    let Some(request) = arp::ArpPacket::parse(payload) else {
        return;
    };
    if request.oper != arp::OPER_REQUEST || request.target_ip != OUR_IP {
        return;
    }
    let Some(our_mac) = rtl8139::mac_address() else {
        return;
    };

    let reply = arp::ArpPacket {
        oper: arp::OPER_REPLY,
        sender_mac: our_mac,
        sender_ip: OUR_IP,
        target_mac: request.sender_mac,
        target_ip: request.sender_ip,
    };

    let mut out = Vec::new();
    eth::EthHeader {
        dst_mac: request.sender_mac,
        src_mac: our_mac,
        ethertype: eth::ETHERTYPE_ARP,
    }
    .write(&mut out);
    reply.write(&mut out);
    rtl8139::send_packet(&out);
}

fn handle_ipv4(header: &eth::EthHeader, payload: &[u8]) {
    let Some((ip, ihl)) = ipv4::Ipv4Header::parse(payload) else {
        return;
    };
    if ip.dst != OUR_IP || ihl + ip.payload_len > payload.len() {
        return;
    }
    let body = &payload[ihl..ihl + ip.payload_len];

    match ip.protocol {
        ipv4::PROTO_ICMP => handle_icmp(header, &ip, body),
        ipv4::PROTO_UDP => handle_udp(header, &ip, body),
        ipv4::PROTO_TCP => handle_tcp(header, &ip, body),
        _ => {}
    }
}

fn handle_icmp(header: &eth::EthHeader, ip: &ipv4::Ipv4Header, body: &[u8]) {
    let Some(echo) = icmp::IcmpEcho::parse_request(body) else {
        return;
    };
    let mut icmp_out = Vec::new();
    echo.write_reply(&mut icmp_out);
    send_ipv4_packet(header.src_mac, ip.src, ipv4::PROTO_ICMP, &icmp_out);
}

/// Servidor de eco UDP (Fase 24): qualquer datagrama que chegar na porta
/// `UDP_ECHO_PORT` volta identico pro remetente (mesma porta de origem
/// vira destino, e vice-versa).
fn handle_udp(header: &eth::EthHeader, ip: &ipv4::Ipv4Header, body: &[u8]) {
    let Some(datagram) = udp::UdpDatagram::parse(body) else {
        return;
    };
    if datagram.dst_port != UDP_ECHO_PORT {
        return;
    }

    let reply = udp::UdpDatagram {
        src_port: UDP_ECHO_PORT,
        dst_port: datagram.src_port,
        payload: datagram.payload,
    };
    let mut udp_out = Vec::new();
    reply.write(&mut udp_out);
    send_ipv4_packet(header.src_mac, ip.src, ipv4::PROTO_UDP, &udp_out);
}

fn send_tcp(dst_mac: [u8; 6], dst_ip: [u8; 4], dst_port: u16, seq: u32, ack: u32, flags: u8, payload: &[u8]) {
    let header = tcp::TcpHeader {
        src_port: TCP_ECHO_PORT,
        dst_port,
        seq,
        ack,
        flags,
        window: 8192,
    };
    let mut tcp_out = Vec::new();
    header.write(OUR_IP, dst_ip, payload, &mut tcp_out);
    send_ipv4_packet(dst_mac, dst_ip, ipv4::PROTO_TCP, &tcp_out);
}

/// Servidor de eco TCP (Fase 25): aceita 1 conexao por vez (handshake de
/// 3 vias), ecoa qualquer dado recebido enquanto ESTABLISHED, fecha
/// graciosamente quando o peer manda FIN -- sem retransmissao/timeout,
/// prova de conceito de handshake+dados+fechamento, nao stack completa.
fn handle_tcp(header: &eth::EthHeader, ip: &ipv4::Ipv4Header, body: &[u8]) {
    let Some((tcp_hdr, data_offset)) = tcp::TcpHeader::parse(body) else {
        return;
    };
    if tcp_hdr.dst_port != TCP_ECHO_PORT {
        return;
    }
    let payload = &body[data_offset..];

    let is_new_syn = tcp_hdr.flags & tcp::FLAG_SYN != 0 && tcp_hdr.flags & tcp::FLAG_ACK == 0;
    if is_new_syn {
        let our_isn: u32 = 1000;
        let peer_seq = tcp_hdr.seq.wrapping_add(1);
        *TCP_CONNECTION.lock() = Some(TcpConnection {
            state: TcpState::SynReceived,
            peer_mac: header.src_mac,
            peer_ip: ip.src,
            peer_port: tcp_hdr.src_port,
            our_seq: our_isn.wrapping_add(1), // o SYN que mandamos consome 1 numero de sequencia
            peer_seq,
        });
        send_tcp(
            header.src_mac,
            ip.src,
            tcp_hdr.src_port,
            our_isn,
            peer_seq,
            tcp::FLAG_SYN | tcp::FLAG_ACK,
            &[],
        );
        return;
    }

    let mut guard = TCP_CONNECTION.lock();
    let Some(conn) = guard.as_mut() else {
        return;
    };
    if conn.peer_ip != ip.src || conn.peer_port != tcp_hdr.src_port {
        return;
    }

    match conn.state {
        TcpState::SynReceived if tcp_hdr.flags & tcp::FLAG_ACK != 0 => {
            conn.state = TcpState::Established;
        }
        TcpState::Established if tcp_hdr.flags & tcp::FLAG_FIN != 0 => {
            conn.peer_seq = conn.peer_seq.wrapping_add(1); // FIN consome 1 numero de sequencia
            let (peer_mac, peer_ip, peer_port, our_seq, peer_seq) =
                (conn.peer_mac, conn.peer_ip, conn.peer_port, conn.our_seq, conn.peer_seq);
            conn.our_seq = conn.our_seq.wrapping_add(1); // nosso FIN tambem consome 1
            conn.state = TcpState::LastAck;
            drop(guard);
            send_tcp(peer_mac, peer_ip, peer_port, our_seq, peer_seq, tcp::FLAG_FIN | tcp::FLAG_ACK, &[]);
        }
        TcpState::Established if !payload.is_empty() => {
            conn.peer_seq = conn.peer_seq.wrapping_add(payload.len() as u32);
            let (peer_mac, peer_ip, peer_port, our_seq, peer_seq) =
                (conn.peer_mac, conn.peer_ip, conn.peer_port, conn.our_seq, conn.peer_seq);
            conn.our_seq = conn.our_seq.wrapping_add(payload.len() as u32);
            drop(guard);
            send_tcp(peer_mac, peer_ip, peer_port, our_seq, peer_seq, tcp::FLAG_ACK | tcp::FLAG_PSH, payload);
        }
        TcpState::LastAck if tcp_hdr.flags & tcp::FLAG_ACK != 0 => {
            drop(guard);
            *TCP_CONNECTION.lock() = None;
        }
        _ => {}
    }
}

/// Task assincrona que fica checando o buffer de RX em loop (mesmo padrao
/// de auto-reenfileiramento de `process::BackgroundTask`, Fase 17) --
/// nunca termina, e' o "servidor" de ARP/ICMP rodando junto com o shell.
pub struct NetPollTask;

impl Future for NetPollTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        if let Some(frame) = rtl8139::try_receive() {
            handle_frame(&frame);
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
