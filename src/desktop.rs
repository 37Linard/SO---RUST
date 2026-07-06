use crate::framebuffer::{set_palette_color, Framebuffer, HEIGHT, WIDTH};
use alloc::vec;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

const BG: u8 = 1;
const TASKBAR: u8 = 2;
const WINDOW_BODY: u8 = 3;
const WINDOW_TITLE: u8 = 4;
const WINDOW_BORDER: u8 = 5;
const CURSOR: u8 = 6;

struct Window {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
}

struct DesktopState {
    fb: Option<Framebuffer>,
    canvas: Vec<u8>,
    window: Window,
    mouse_x: i32,
    mouse_y: i32,
    dragging: bool,
    drag_offset: (i32, i32),
    left_down: bool,
}

impl DesktopState {
    fn new() -> Self {
        DesktopState {
            fb: None,
            canvas: vec![BG; WIDTH * HEIGHT],
            window: Window {
                x: 60,
                y: 30,
                w: 200,
                h: 110,
            },
            mouse_x: (WIDTH / 2) as i32,
            mouse_y: (HEIGHT / 2) as i32,
            dragging: false,
            drag_offset: (0, 0),
            left_down: false,
        }
    }
}

lazy_static! {
    static ref DESKTOP: Mutex<DesktopState> = Mutex::new(DesktopState::new());
}

fn put(canvas: &mut [u8], x: i32, y: i32, color: u8) {
    if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT {
        return;
    }
    canvas[y as usize * WIDTH + x as usize] = color;
}

fn fill(canvas: &mut [u8], x: i32, y: i32, w: usize, h: usize, color: u8) {
    for row in 0..h as i32 {
        for col in 0..w as i32 {
            put(canvas, x + col, y + row, color);
        }
    }
}

fn rect(canvas: &mut [u8], x: i32, y: i32, w: usize, h: usize, color: u8) {
    for col in 0..w as i32 {
        put(canvas, x + col, y, color);
        put(canvas, x + col, y + h as i32 - 1, color);
    }
    for row in 0..h as i32 {
        put(canvas, x, y + row, color);
        put(canvas, x + w as i32 - 1, y + row, color);
    }
}

pub fn setup_palette() {
    set_palette_color(BG, 8, 30, 35);
    set_palette_color(TASKBAR, 6, 6, 8);
    set_palette_color(WINDOW_BODY, 50, 50, 53);
    set_palette_color(WINDOW_TITLE, 8, 12, 40);
    set_palette_color(WINDOW_BORDER, 63, 63, 63);
    set_palette_color(CURSOR, 63, 20, 20);
}

pub fn init(fb: Framebuffer) {
    DESKTOP.lock().fb = Some(fb);
    render();
}

fn render() {
    let mut d = DESKTOP.lock();

    for p in d.canvas.iter_mut() {
        *p = BG;
    }
    fill(&mut d.canvas, 0, HEIGHT as i32 - 16, WIDTH, 16, TASKBAR);

    let (wx, wy, ww, wh) = (d.window.x, d.window.y, d.window.w, d.window.h);
    fill(&mut d.canvas, wx, wy, ww, wh, WINDOW_BODY);
    fill(&mut d.canvas, wx, wy, ww, 12, WINDOW_TITLE);
    rect(&mut d.canvas, wx, wy, ww, wh, WINDOW_BORDER);

    let (mouse_x, mouse_y) = (d.mouse_x, d.mouse_y);

    if let Some(fb) = d.fb.as_ref() {
        for i in 0..WIDTH * HEIGHT {
            fb.put_pixel(i % WIDTH, i / WIDTH, d.canvas[i]);
        }
        // cursor desenhado por cima direto no hardware -- nao entra no canvas,
        // entao no proximo render() ele "some" sozinho sem deixar rastro.
        if mouse_x >= 0 && mouse_y >= 0 {
            fb.put_pixel(mouse_x as usize, mouse_y as usize, CURSOR);
        }
    }
}

pub fn handle_mouse(dx: i32, dy: i32, left: bool) {
    {
        let mut d = DESKTOP.lock();
        d.mouse_x = (d.mouse_x + dx).clamp(0, WIDTH as i32 - 1);
        d.mouse_y = (d.mouse_y - dy).clamp(0, HEIGHT as i32 - 1); // eixo Y do PS/2 e invertido

        let (mx, my) = (d.mouse_x, d.mouse_y);
        let (wx, wy, ww) = (d.window.x, d.window.y, d.window.w);

        if left && !d.left_down {
            let in_titlebar = mx >= wx && mx < wx + ww as i32 && my >= wy && my < wy + 12;
            if in_titlebar {
                d.dragging = true;
                d.drag_offset = (mx - wx, my - wy);
            }
        } else if !left {
            d.dragging = false;
        }
        d.left_down = left;

        if d.dragging {
            d.window.x = mx - d.drag_offset.0;
            d.window.y = my - d.drag_offset.1;
        }
    }

    render();
}
