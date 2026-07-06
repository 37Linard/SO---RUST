use x86_64::instructions::port::Port;

// Barramento ATA primario, drive SLAVE -- e' o que o flag `-hdb` do QEMU
// conecta (o disco de boot ja ocupa o master do mesmo barramento). Mesmas
// portas de sempre (0x1F0-0x1F7), so muda o bit de selecao de drive.
const PORT_DATA: u16 = 0x1F0;
const PORT_SECTOR_COUNT: u16 = 0x1F2;
const PORT_LBA_LOW: u16 = 0x1F3;
const PORT_LBA_MID: u16 = 0x1F4;
const PORT_LBA_HIGH: u16 = 0x1F5;
const PORT_DRIVE_HEAD: u16 = 0x1F6;
const PORT_STATUS_COMMAND: u16 = 0x1F7;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_CACHE_FLUSH: u8 = 0xE7;

const STATUS_BSY: u8 = 0x80;
const STATUS_DRQ: u8 = 0x08;

/// Drive slave, modo LBA (bit 6 = LBA, bit 4 = slave, bit 5/7 sempre 1
/// por convencao historica do registrador drive/head).
const DRIVE_SLAVE_LBA: u8 = 0xF0;

fn wait_not_busy() {
    let mut status_port: Port<u8> = Port::new(PORT_STATUS_COMMAND);
    loop {
        let status: u8 = unsafe { status_port.read() };
        if status & STATUS_BSY == 0 {
            break;
        }
    }
}

fn wait_data_ready() {
    let mut status_port: Port<u8> = Port::new(PORT_STATUS_COMMAND);
    loop {
        let status: u8 = unsafe { status_port.read() };
        if status & STATUS_BSY == 0 && status & STATUS_DRQ != 0 {
            break;
        }
    }
}

fn select_sector(lba: u32) {
    let mut drive_head: Port<u8> = Port::new(PORT_DRIVE_HEAD);
    let mut sector_count: Port<u8> = Port::new(PORT_SECTOR_COUNT);
    let mut lba_low: Port<u8> = Port::new(PORT_LBA_LOW);
    let mut lba_mid: Port<u8> = Port::new(PORT_LBA_MID);
    let mut lba_high: Port<u8> = Port::new(PORT_LBA_HIGH);

    unsafe {
        drive_head.write(DRIVE_SLAVE_LBA | (((lba >> 24) & 0x0F) as u8));
        sector_count.write(1u8);
        lba_low.write((lba & 0xFF) as u8);
        lba_mid.write(((lba >> 8) & 0xFF) as u8);
        lba_high.write(((lba >> 16) & 0xFF) as u8);
    }
}

/// Le um setor de 512 bytes do disco secundario (`-hdb`) via PIO, por
/// polling (sem interrupcao de disco -- mais simples, suficiente pra
/// provar persistencia de verdade sob QEMU).
pub fn read_sector(lba: u32, buf: &mut [u8; 512]) {
    wait_not_busy();
    select_sector(lba);

    let mut command_port: Port<u8> = Port::new(PORT_STATUS_COMMAND);
    unsafe { command_port.write(CMD_READ_SECTORS) };

    wait_data_ready();

    let mut data_port: Port<u16> = Port::new(PORT_DATA);
    for i in 0..256 {
        let word: u16 = unsafe { data_port.read() };
        buf[i * 2] = (word & 0xFF) as u8;
        buf[i * 2 + 1] = (word >> 8) as u8;
    }
}

/// Escreve um setor de 512 bytes no disco secundario (`-hdb`) via PIO,
/// seguido de CACHE FLUSH -- garante que o setor realmente foi pro disco
/// antes de continuar (importante pro write-through do filesystem).
pub fn write_sector(lba: u32, buf: &[u8; 512]) {
    wait_not_busy();
    select_sector(lba);

    let mut command_port: Port<u8> = Port::new(PORT_STATUS_COMMAND);
    unsafe { command_port.write(CMD_WRITE_SECTORS) };

    wait_data_ready();

    let mut data_port: Port<u16> = Port::new(PORT_DATA);
    for i in 0..256 {
        let word = buf[i * 2] as u16 | ((buf[i * 2 + 1] as u16) << 8);
        unsafe { data_port.write(word) };
    }

    wait_not_busy();
    unsafe { command_port.write(CMD_CACHE_FLUSH) };
    wait_not_busy();
}
