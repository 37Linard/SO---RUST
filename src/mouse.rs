use crate::println;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::instructions::port::Port;

struct PacketState {
    packet: [u8; 3],
    index: u8,
}

lazy_static! {
    static ref PACKET: Mutex<PacketState> = Mutex::new(PacketState {
        packet: [0; 3],
        index: 0,
    });
}

fn wait_input_clear() {
    let mut status_port: Port<u8> = Port::new(0x64);
    loop {
        let status: u8 = unsafe { status_port.read() };
        if status & 0b10 == 0 {
            break;
        }
    }
}

fn wait_output_full() {
    let mut status_port: Port<u8> = Port::new(0x64);
    loop {
        let status: u8 = unsafe { status_port.read() };
        if status & 0b1 != 0 {
            break;
        }
    }
}

fn mouse_write(value: u8) {
    let mut command_port: Port<u8> = Port::new(0x64);
    let mut data_port: Port<u8> = Port::new(0x60);
    unsafe {
        wait_input_clear();
        command_port.write(0xD4u8);
        wait_input_clear();
        data_port.write(value);
    }
}

fn mouse_read() -> u8 {
    wait_output_full();
    let mut data_port: Port<u8> = Port::new(0x60);
    unsafe { data_port.read() }
}

pub fn init() {
    let mut command_port: Port<u8> = Port::new(0x64);
    let mut data_port: Port<u8> = Port::new(0x60);

    unsafe {
        wait_input_clear();
        command_port.write(0xA8u8); // habilita porta auxiliar (mouse)

        wait_input_clear();
        command_port.write(0x20u8); // pede pra ler o byte de configuracao
        let mut config = mouse_read();
        config |= 0b0000_0010; // bit1: habilita IRQ12
        config &= !0b0010_0000; // bit5: 0 = habilita clock do mouse

        wait_input_clear();
        command_port.write(0x60u8); // vai escrever o byte de configuracao
        wait_input_clear();
        data_port.write(config);
    }

    mouse_write(0xF6); // usa configuracao padrao
    mouse_read(); // ACK

    mouse_write(0xF4); // habilita streaming de pacotes
    mouse_read(); // ACK

    // desmascara IRQ12 (linha 4 do PIC escravo) e IRQ2 no mestre (cascata --
    // sem isso, NENHUMA interrupcao do PIC escravo chega na CPU, mesmo com
    // a linha correta desmascarada no escravo)
    unsafe {
        let mut slave_data: Port<u8> = Port::new(0xA1);
        let slave_mask = slave_data.read();
        slave_data.write(slave_mask & !0b0001_0000);

        let mut master_data: Port<u8> = Port::new(0x21);
        let master_mask = master_data.read();
        master_data.write(master_mask & !0b0000_0100);
    }

    println!("Mouse PS/2 inicializado.");
}

pub fn handle_byte(byte: u8) {
    let mut state = PACKET.lock();
    let index = state.index as usize;
    state.packet[index] = byte;
    state.index += 1;

    if state.index < 3 {
        return;
    }
    state.index = 0;

    let flags = state.packet[0];
    // bit3 sempre 1 num pacote alinhado; se nao estiver, descarta e resincroniza
    if flags & 0b0000_1000 == 0 {
        return;
    }

    let mut dx = state.packet[1] as i32;
    let mut dy = state.packet[2] as i32;
    if flags & 0b0001_0000 != 0 {
        dx -= 256;
    }
    if flags & 0b0010_0000 != 0 {
        dy -= 256;
    }
    let left = flags & 0b0000_0001 != 0;
    drop(state);

    crate::desktop::handle_mouse(dx, dy, left);
}
