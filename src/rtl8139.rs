use crate::pci;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::port::Port;
use x86_64::VirtAddr;

const VENDOR_ID: u16 = 0x10EC;
const DEVICE_ID: u16 = 0x8139;

// Offsets de registrador (relativos ao BAR0, mapa classico do rtl8139 --
// ver nota no plano da Fase 23: conferido/ajustado empiricamente, nao e'
// algo que da pra "desenhar" com confianca total como a fonte da Fase 21).
const REG_IDR0: u16 = 0x00; // MAC (6 bytes)
const REG_TSD0: u16 = 0x10; // status/tamanho dos 4 descritores de TX
const REG_TSAD0: u16 = 0x20; // endereco fisico dos 4 buffers de TX
const REG_RBSTART: u16 = 0x30;
const REG_CAPR: u16 = 0x38;
#[allow(dead_code)]
const REG_CBR: u16 = 0x3A; // guardado por completude do mapa de registrador, nao usado (so' diagnostico)
const REG_CR: u16 = 0x37;
const REG_IMR: u16 = 0x3C;
const REG_RCR: u16 = 0x44;
const REG_CONFIG1: u16 = 0x52;

const CR_RST: u8 = 0x10;
const CR_RE: u8 = 0x08;
const CR_TE: u8 = 0x04;
const CR_BUFE: u8 = 0x01;

const RCR_AAP: u32 = 0x01; // aceita todos os pacotes (promiscuo)
const RCR_APM: u32 = 0x02; // aceita pacotes com nosso MAC
const RCR_AM: u32 = 0x04; // aceita multicast
const RCR_AB: u32 = 0x08; // aceita broadcast

/// Tamanho nominal do anel (bate com RBLEN=00 no RCR, "8K"). O buffer
/// fisico alocado e' maior (`RX_BUFFER_LEN`) pra dar espaco de sobra
/// (already permitido por deixar o bit WRAP do RCR desligado) pro ultimo
/// pacote antes da volta nao precisar ser cortado no meio.
const RX_RING_SIZE: usize = 8192;
const RX_BUFFER_LEN: usize = RX_RING_SIZE + 16 + 1500;
const TX_BUFFER_LEN: usize = 1536;
const TX_BUFFER_COUNT: usize = 4;

static mut RX_BUFFER: [u8; RX_BUFFER_LEN] = [0; RX_BUFFER_LEN];
static mut TX_BUFFERS: [[u8; TX_BUFFER_LEN]; TX_BUFFER_COUNT] = [[0; TX_BUFFER_LEN]; TX_BUFFER_COUNT];

struct Rtl8139 {
    io_base: u16,
    mac: [u8; 6],
    rx_offset: usize,
    next_tx: usize,
}

static DEVICE: Mutex<Option<Rtl8139>> = Mutex::new(None);

fn port_u8(base: u16, reg: u16) -> Port<u8> {
    Port::new(base + reg)
}
fn port_u16(base: u16, reg: u16) -> Port<u16> {
    Port::new(base + reg)
}
fn port_u32(base: u16, reg: u16) -> Port<u32> {
    Port::new(base + reg)
}

/// Acha o rtl8139 no barramento PCI, reseta e configura ele (buffers de
/// RX/TX estaticos do kernel, sem interrupcao -- ver decisao de design na
/// Fase 23). Retorna o MAC lido, pra confirmar visualmente que a leitura
/// de registrador esta funcionando (endereco tudo-zero ou tudo-FF indica
/// offset errado).
pub fn init() -> Result<[u8; 6], &'static str> {
    let dev = pci::find_device(VENDOR_ID, DEVICE_ID).ok_or("rtl8139 nao encontrado no PCI")?;
    pci::enable_bus_mastering(&dev);

    let io_base = (dev.bar0 & 0xFFFF) as u16;

    unsafe {
        // Liga o cartao (sai de estado de baixo consumo/sleep).
        port_u8(io_base, REG_CONFIG1).write(0x00u8);

        // Reset por software -- o bit RST se limpa sozinho quando termina.
        port_u8(io_base, REG_CR).write(CR_RST);
        let mut spins: u64 = 0;
        loop {
            let cr: u8 = port_u8(io_base, REG_CR).read();
            if cr & CR_RST == 0 {
                break;
            }
            spins += 1;
            if spins > 10_000_000 {
                return Err("timeout no reset do rtl8139");
            }
        }

        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = port_u8(io_base, REG_IDR0 + i as u16).read();
        }

        let mapper_guard = crate::process::KERNEL_MEMORY.lock();
        let rx_phys = {
            let (mapper, _) = mapper_guard
                .as_ref()
                .ok_or("memoria do kernel indisponivel")?;
            let rx_virt = VirtAddr::new(core::ptr::addr_of!(RX_BUFFER) as u64);
            translate(mapper, rx_virt).ok_or("falha ao traduzir endereco do buffer de RX")?
        };
        drop(mapper_guard);

        port_u32(io_base, REG_RBSTART).write(rx_phys as u32);

        port_u16(io_base, REG_IMR).write(0x0000u16); // sem interrupcao -- so' polling

        let rcr = RCR_AAP | RCR_APM | RCR_AM | RCR_AB;
        port_u32(io_base, REG_RCR).write(rcr);

        port_u8(io_base, REG_CR).write(CR_RE | CR_TE);

        *DEVICE.lock() = Some(Rtl8139 {
            io_base,
            mac,
            rx_offset: 0,
            next_tx: 0,
        });

        Ok(mac)
    }
}

fn translate(mapper: &x86_64::structures::paging::OffsetPageTable<'static>, addr: VirtAddr) -> Option<u64> {
    use x86_64::structures::paging::Translate;
    mapper.translate_addr(addr).map(|p| p.as_u64())
}

/// Manda um frame Ethernet (pronto, com cabecalho) pro fio. Copia pro
/// proximo buffer de TX (round-robin entre os 4) e escreve o tamanho no
/// descritor correspondente -- isso e' o que dispara a transmissao.
pub fn send_packet(frame: &[u8]) {
    let mut guard = DEVICE.lock();
    let Some(dev) = guard.as_mut() else { return };

    let slot = dev.next_tx;
    dev.next_tx = (dev.next_tx + 1) % TX_BUFFER_COUNT;
    let io_base = dev.io_base;

    unsafe {
        let buf = &mut TX_BUFFERS[slot];
        let len = frame.len().min(TX_BUFFER_LEN);
        buf[..len].copy_from_slice(&frame[..len]);

        let mapper_guard = crate::process::KERNEL_MEMORY.lock();
        let Some((mapper, _)) = mapper_guard.as_ref() else {
            return;
        };
        let tx_virt = VirtAddr::new(core::ptr::addr_of!(TX_BUFFERS[slot]) as u64);
        let Some(tx_phys) = translate(mapper, tx_virt) else {
            return;
        };
        drop(mapper_guard);

        port_u32(io_base, REG_TSAD0 + (slot as u16) * 4).write(tx_phys as u32);
        port_u32(io_base, REG_TSD0 + (slot as u16) * 4).write(len as u32);
    }
}

/// Confere se tem pacote novo no buffer de RX (bit BUFE do registrador de
/// comando = 1 quando o buffer esta vazio) e, se tiver, copia o frame
/// Ethernet completo (sem o cabecalho de 4 bytes do anel nem o CRC final)
/// e avanca o ponteiro de leitura. Chamado em loop por `net::poll_task`.
pub fn try_receive() -> Option<Vec<u8>> {
    let mut guard = DEVICE.lock();
    let dev = guard.as_mut()?;
    let io_base = dev.io_base;

    unsafe {
        let cr: u8 = port_u8(io_base, REG_CR).read();
        if cr & CR_BUFE != 0 {
            return None; // buffer vazio, nada novo
        }

        let offset = dev.rx_offset;
        let header = u16::from_le_bytes([RX_BUFFER[offset], RX_BUFFER[offset + 1]]);
        let status_ok = header & 0x01 != 0;
        let length = u16::from_le_bytes([RX_BUFFER[offset + 2], RX_BUFFER[offset + 3]]) as usize;

        if !status_ok || length < 4 || length > RX_BUFFER_LEN {
            // pacote invalido -- nao devia acontecer em uso normal, mas
            // evita ler lixo/travar se o parsing de offset estiver errado.
            return None;
        }

        let frame_len = length - 4; // remove os 4 bytes de CRC no final
        let data_start = offset + 4;
        let mut frame = Vec::with_capacity(frame_len);
        for i in 0..frame_len {
            frame.push(RX_BUFFER[(data_start + i) % RX_BUFFER_LEN]);
        }

        // proximo pacote comeca logo depois do header(4)+length atual,
        // arredondado pra cima num multiplo de 4. O anel logico e'
        // RX_RING_SIZE (o espaco extra no buffer fisico e' so' pra um
        // pacote nao precisar ser cortado bem em cima da volta).
        let mut next_offset = (offset + 4 + length + 3) & !3;
        if next_offset >= RX_RING_SIZE {
            next_offset -= RX_RING_SIZE;
        }
        dev.rx_offset = next_offset;

        // Quirk conhecido do rtl8139: CAPR precisa ser escrito 0x10 (16)
        // bytes ANTES do proximo offset de leitura de verdade.
        let capr = next_offset.wrapping_sub(0x10) as u16;
        port_u16(io_base, REG_CAPR).write(capr);

        Some(frame)
    }
}

pub fn mac_address() -> Option<[u8; 6]> {
    DEVICE.lock().as_ref().map(|d| d.mac)
}
