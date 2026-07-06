use x86_64::instructions::port::Port;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 200;

/// Framebuffer VGA modo 13h (linear, 1 byte por pixel = indice de paleta).
/// O bootloader (feature `vga_320x200`) ja trocou pro modo grafico antes do
/// kernel iniciar; o buffer fisico fica em 0xa0000. Como usamos a feature
/// `map_physical_memory`, toda memoria fisica esta acessivel em
/// `physical_memory_offset + endereco_fisico`.
pub struct Framebuffer {
    base: *mut u8,
}

unsafe impl Send for Framebuffer {}

impl Framebuffer {
    pub fn new(physical_memory_offset: u64) -> Self {
        Framebuffer {
            base: (physical_memory_offset + 0xa0000) as *mut u8,
        }
    }

    pub fn put_pixel(&self, x: usize, y: usize, color: u8) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let offset = y * WIDTH + x;
        unsafe {
            core::ptr::write_volatile(self.base.add(offset), color);
        }
    }

    pub fn clear(&self, color: u8) {
        for i in 0..WIDTH * HEIGHT {
            unsafe {
                core::ptr::write_volatile(self.base.add(i), color);
            }
        }
    }

    pub fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u8) {
        for row in y..(y + h).min(HEIGHT) {
            for col in x..(x + w).min(WIDTH) {
                self.put_pixel(col, row, color);
            }
        }
    }

    pub fn draw_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u8) {
        for col in x..(x + w).min(WIDTH) {
            self.put_pixel(col, y, color);
            if y + h > 0 {
                self.put_pixel(col, (y + h - 1).min(HEIGHT - 1), color);
            }
        }
        for row in y..(y + h).min(HEIGHT) {
            self.put_pixel(x, row, color);
            if x + w > 0 {
                self.put_pixel((x + w - 1).min(WIDTH - 1), row, color);
            }
        }
    }
}

/// Reprograma uma entrada da paleta VGA (DAC). r/g/b vao de 0 a 63 (6 bits).
pub fn set_palette_color(index: u8, r: u8, g: u8, b: u8) {
    let mut index_port: Port<u8> = Port::new(0x3C8);
    let mut data_port: Port<u8> = Port::new(0x3C9);
    unsafe {
        index_port.write(index);
        data_port.write(r);
        data_port.write(g);
        data_port.write(b);
    }
}
