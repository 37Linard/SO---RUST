use x86_64::instructions::port::Port;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub bar0: u32,
    /// Lida por completude/diagnostico -- o driver de rede usa polling
    /// (ver `rtl8139.rs`), entao nunca precisa registrar handler pra IRQ.
    #[allow(dead_code)]
    pub interrupt_line: u8,
}

fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC)
}

fn read_config_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let mut addr_port: Port<u32> = Port::new(CONFIG_ADDRESS);
    let mut data_port: Port<u32> = Port::new(CONFIG_DATA);
    unsafe {
        addr_port.write(config_address(bus, device, function, offset));
        data_port.read()
    }
}

/// Varre o barramento 0 (suficiente pro layout padrao do QEMU, sem
/// bridges/multiplos barramentos) procurando um dispositivo com o
/// vendor/device id dados. Retorna o BAR0 (base de I/O, ja com os 2 bits
/// baixos de flag removidos) e a linha de IRQ (lida por completude --
/// o driver de rede usa polling, nao interrupcao, ver `rtl8139.rs`).
pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    for device in 0..32 {
        for function in 0..8 {
            let id = read_config_u32(0, device, function, 0x00);
            let vendor = (id & 0xFFFF) as u16;
            if vendor == 0xFFFF {
                continue; // nenhum dispositivo nessa posicao
            }
            let dev_id = (id >> 16) as u16;
            if vendor == vendor_id && dev_id == device_id {
                let bar0 = read_config_u32(0, device, function, 0x10) & 0xFFFF_FFFC;
                let interrupt_line = (read_config_u32(0, device, function, 0x3C) & 0xFF) as u8;
                return Some(PciDevice {
                    bus: 0,
                    device,
                    function,
                    bar0,
                    interrupt_line,
                });
            }
        }
    }
    None
}

/// Liga o bit "bus master" (bit2) no registrador de comando do PCI --
/// sem isso o dispositivo nao consegue fazer DMA (ler/escrever memoria
/// do sistema sozinho), necessario pro rtl8139 mandar/receber pacotes.
pub fn enable_bus_mastering(dev: &PciDevice) {
    // O offset 0x04 forma um dword junto com o registrador de status
    // (0x06) -- le o dword completo, troca so' os 16 bits baixos
    // (comando), escreve de volta inteiro.
    let full = read_config_u32(dev.bus, dev.device, dev.function, 0x04);
    let merged = full | 0x0004;

    let mut addr_port: Port<u32> = Port::new(CONFIG_ADDRESS);
    let mut data_port: Port<u32> = Port::new(CONFIG_DATA);
    unsafe {
        addr_port.write(config_address(dev.bus, dev.device, dev.function, 0x04));
        data_port.write(merged);
    }
}
