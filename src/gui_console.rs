use core::fmt;
use lazy_static::lazy_static;
use spin::Mutex;

pub const COLS: usize = 25;
pub const ROWS: usize = 12;

/// Console de texto pra dentro da janela terminal do compositor (Fase 22)
/// -- mesmo modelo simples que `vga_buffer::Writer` ja usa (sempre escreve
/// na ultima linha da grade, `\n` sempre rola tudo pra cima uma linha),
/// so' que guarda os caracteres numa grade em memoria em vez de escrever
/// direto no buffer de texto da VGA. `src/desktop.rs` que le essa grade
/// e desenha ela de verdade via `font8x8`.
struct GuiConsole {
    chars: [[u8; COLS]; ROWS],
    /// Onde o proximo caractere escrito vai parar (equivalente a
    /// `column_position` do `vga_buffer::Writer`).
    column: usize,
    /// Posicao visual do cursor de edicao -- pode ser diferente de
    /// `column` (setas esquerda/direita movem isso sem tocar no texto).
    cursor_col: usize,
}

impl GuiConsole {
    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column >= COLS {
                    self.new_line();
                }
                let row = ROWS - 1;
                self.chars[row][self.column] = byte;
                self.column += 1;
            }
        }
    }

    fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                _ => self.write_byte(b'?'),
            }
        }
    }

    fn new_line(&mut self) {
        for row in 1..ROWS {
            self.chars[row - 1] = self.chars[row];
        }
        self.clear_row(ROWS - 1);
        self.column = 0;
    }

    fn clear_row(&mut self, row: usize) {
        self.chars[row] = [b' '; COLS];
    }
}

impl fmt::Write for GuiConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    static ref CONSOLE: Mutex<GuiConsole> = Mutex::new(GuiConsole {
        chars: [[b' '; COLS]; ROWS],
        column: 0,
        cursor_col: 0,
    });
}

pub fn print(args: fmt::Arguments) {
    use core::fmt::Write;
    CONSOLE.lock().write_fmt(args).unwrap();
    crate::desktop::request_redraw();
}

pub fn clear_screen() {
    let mut console = CONSOLE.lock();
    for row in 0..ROWS {
        console.clear_row(row);
    }
    console.column = 0;
    drop(console);
    crate::desktop::request_redraw();
}

pub fn redraw_line(prompt_len: usize, text: &str) {
    let mut console = CONSOLE.lock();
    let row = ROWS - 1;
    for (i, byte) in text.bytes().enumerate() {
        let col = prompt_len + i;
        if col >= COLS {
            break;
        }
        console.chars[row][col] = byte;
    }
    for col in (prompt_len + text.len()).min(COLS)..COLS {
        console.chars[row][col] = b' ';
    }
    console.column = (prompt_len + text.len()).min(COLS);
    drop(console);
    crate::desktop::request_redraw();
}

pub fn set_cursor(col: usize) {
    let mut console = CONSOLE.lock();
    console.cursor_col = col.min(COLS - 1);
    drop(console);
    crate::desktop::request_redraw();
}

/// Le o conteudo atual da grade + posicao do cursor -- usado por
/// `desktop::render()` pra desenhar o texto de verdade na janela terminal.
pub fn snapshot() -> ([[u8; COLS]; ROWS], usize) {
    let console = CONSOLE.lock();
    (console.chars, console.cursor_col)
}
