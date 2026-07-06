use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;

/// Fase 22: quando o modo grafico (`gui`) esta ativo, `print!`/`println!`
/// e as chamadas diretas do shell (`redraw_line`/`set_cursor`/
/// `clear_screen`) passam a desenhar na janela terminal do compositor
/// (`src/gui_console.rs`) em vez do buffer de texto da VGA -- shell.rs
/// nao precisa saber a diferenca, so' essas 4 funcoes redirecionam.
static GUI_MODE: AtomicBool = AtomicBool::new(false);

pub fn enable_gui_mode() {
    GUI_MODE.store(true, Ordering::Relaxed);
}

pub fn is_gui_mode() -> bool {
    GUI_MODE.load(Ordering::Relaxed)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub struct Writer {
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = BUFFER_HEIGHT - 1;
                let col = self.column_position;

                let color_code = self.color_code;
                self.buffer.chars[row][col].write(ScreenChar {
                    ascii_character: byte,
                    color_code,
                });
                self.column_position += 1;
            }
        }
    }

    pub fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                _ => self.write_byte(0xfe),
            }
        }
    }

    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character = self.buffer.chars[row][col].read();
                self.buffer.chars[row - 1][col].write(character);
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer {
        column_position: 0,
        color_code: ColorCode::new(Color::LightCyan, Color::Black),
        buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
    });
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::vga_buffer::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    if GUI_MODE.load(Ordering::Relaxed) {
        crate::gui_console::print(args);
        return;
    }

    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        WRITER.lock().write_fmt(args).unwrap();
    });
}

pub fn clear_screen() {
    if GUI_MODE.load(Ordering::Relaxed) {
        crate::gui_console::clear_screen();
        return;
    }

    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        let mut writer = WRITER.lock();
        for row in 0..BUFFER_HEIGHT {
            writer.clear_row(row);
        }
        writer.column_position = 0;
    });
}

/// Reescreve a linha de edicao (ultima linha da tela) a partir da coluna
/// `prompt_len` com o conteudo de `text`, limpando qualquer sobra da
/// versao anterior (texto novo menor que o antigo). Usado pelo shell
/// (Fase 16, edicao de linha) toda vez que o buffer ou o cursor mudam --
/// mais simples e robusto que tentar inserir/apagar so o trecho que mudou.
pub fn redraw_line(prompt_len: usize, text: &str) {
    if GUI_MODE.load(Ordering::Relaxed) {
        crate::gui_console::redraw_line(prompt_len, text);
        return;
    }

    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        let mut writer = WRITER.lock();
        let row = BUFFER_HEIGHT - 1;
        let color_code = writer.color_code;

        for (i, byte) in text.bytes().enumerate() {
            let col = prompt_len + i;
            if col >= BUFFER_WIDTH {
                break;
            }
            writer.buffer.chars[row][col].write(ScreenChar {
                ascii_character: byte,
                color_code,
            });
        }
        for col in (prompt_len + text.len()).min(BUFFER_WIDTH)..BUFFER_WIDTH {
            writer.buffer.chars[row][col].write(ScreenChar {
                ascii_character: b' ',
                color_code,
            });
        }
        writer.column_position = (prompt_len + text.len()).min(BUFFER_WIDTH);
    });
}

/// Move o cursor de hardware da VGA (registradores do CRTC, portas
/// 0x3D4/0x3D5) pra coluna `col` da ultima linha -- sem isso, mover o
/// cursor logico (setas) nao teria nenhum indicador visual de onde a
/// edicao esta acontecendo.
pub fn set_cursor(col: usize) {
    if GUI_MODE.load(Ordering::Relaxed) {
        crate::gui_console::set_cursor(col);
        return;
    }

    use x86_64::instructions::interrupts;
    use x86_64::instructions::port::Port;

    interrupts::without_interrupts(|| {
        let pos = (BUFFER_HEIGHT - 1) * BUFFER_WIDTH + col.min(BUFFER_WIDTH - 1);
        let mut index_port: Port<u8> = Port::new(0x3D4);
        let mut data_port: Port<u8> = Port::new(0x3D5);
        unsafe {
            index_port.write(0x0F_u8);
            data_port.write((pos & 0xFF) as u8);
            index_port.write(0x0E_u8);
            data_port.write(((pos >> 8) & 0xFF) as u8);
        }
    });
}

