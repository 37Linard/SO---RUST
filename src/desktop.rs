use crate::font8x8;
use crate::framebuffer::{set_palette_color, Framebuffer, HEIGHT, WIDTH};
use crate::gui_console;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

const BG: u8 = 1;
const TASKBAR: u8 = 2;
const WINDOW_TITLE: u8 = 4;
const WINDOW_BORDER: u8 = 5;
const CURSOR: u8 = 6;
const CLOSE_BTN: u8 = 7;
const MINIMIZE_BTN: u8 = 8;
const NEWWIN_TILE: u8 = 9;
/// Corpo de cada janela cicla entre essas 4 cores -- o titulo (Fase 21, ver
/// `font8x8.rs`) ja diferencia por texto, mas manter cores variadas ajuda
/// a ver a sobreposicao/z-order de longe.
const BODY_COLORS: [u8; 4] = [3, 10, 11, 12];
const TEXT_COLOR: u8 = WINDOW_BORDER; // reaproveita a cor branca, bom contraste
const CONSOLE_BG: u8 = 13;
const TERMINAL_PAD: i32 = 2;

const TITLEBAR_HEIGHT: i32 = 12;
const BUTTON_SIZE: i32 = 8;
const BUTTON_MARGIN: i32 = 2;
const TASKBAR_HEIGHT: i32 = 16;
const TASKBAR_TILE_W: i32 = 14;
const TASKBAR_TILE_H: i32 = 12;
const TASKBAR_TILE_GAP: i32 = 4;

const DEFAULT_WIN_W: usize = 110;
const DEFAULT_WIN_H: usize = 70;
const CASCADE_STEP: i32 = 15;
const CASCADE_WRAP: i32 = 6;

struct Window {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    body_color: u8,
    minimized: bool,
    label: String,
    /// Janela terminal (Fase 22): em vez de corpo solido, desenha o
    /// conteudo de `gui_console` (texto de verdade do shell) por cima.
    is_terminal: bool,
}

impl Window {
    fn close_button_rect(&self) -> (i32, i32, i32, i32) {
        let x = self.x + self.w as i32 - BUTTON_SIZE - BUTTON_MARGIN;
        (x, self.y + BUTTON_MARGIN, BUTTON_SIZE, BUTTON_SIZE)
    }

    fn minimize_button_rect(&self) -> (i32, i32, i32, i32) {
        let x = self.x + self.w as i32 - 2 * BUTTON_SIZE - 2 * BUTTON_MARGIN;
        (x, self.y + BUTTON_MARGIN, BUTTON_SIZE, BUTTON_SIZE)
    }
}

fn point_in(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

struct DesktopState {
    fb: Option<Framebuffer>,
    canvas: Vec<u8>,
    windows: Vec<Window>,
    next_id: usize,
    mouse_x: i32,
    mouse_y: i32,
    /// Indice (dentro de `windows`) da janela sendo arrastada, se houver.
    dragging: Option<usize>,
    drag_offset: (i32, i32),
    left_down: bool,
}

impl DesktopState {
    fn new() -> Self {
        let mut state = DesktopState {
            fb: None,
            canvas: vec![BG; WIDTH * HEIGHT],
            windows: Vec::new(),
            next_id: 0,
            mouse_x: (WIDTH / 2) as i32,
            mouse_y: (HEIGHT / 2) as i32,
            dragging: None,
            drag_offset: (0, 0),
            left_down: false,
        };
        state.spawn_window();
        state.spawn_window();
        state.spawn_terminal_window();
        state
    }

    fn spawn_window(&mut self) {
        let cascade = (self.next_id as i32) % CASCADE_WRAP;
        let id = self.next_id;
        self.next_id += 1;
        self.windows.push(Window {
            x: 20 + cascade * CASCADE_STEP,
            y: 10 + cascade * CASCADE_STEP,
            w: DEFAULT_WIN_W,
            h: DEFAULT_WIN_H,
            body_color: BODY_COLORS[id % BODY_COLORS.len()],
            minimized: false,
            label: format!("JANELA {}", id + 1),
            is_terminal: false,
        });
    }

    fn spawn_terminal_window(&mut self) {
        let w = 2 * TERMINAL_PAD + gui_console::COLS as i32 * font8x8::CHAR_WIDTH;
        let h = TITLEBAR_HEIGHT + 2 * TERMINAL_PAD + gui_console::ROWS as i32 * font8x8::CHAR_HEIGHT;
        self.windows.push(Window {
            x: 10,
            y: 5,
            w: w as usize,
            h: h as usize,
            body_color: CONSOLE_BG,
            minimized: false,
            label: String::from("SHELL"),
            is_terminal: true,
        });
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

fn fill(canvas: &mut [u8], x: i32, y: i32, w: i32, h: i32, color: u8) {
    for row in 0..h {
        for col in 0..w {
            put(canvas, x + col, y + row, color);
        }
    }
}

fn rect(canvas: &mut [u8], x: i32, y: i32, w: i32, h: i32, color: u8) {
    for col in 0..w {
        put(canvas, x + col, y, color);
        put(canvas, x + col, y + h - 1, color);
    }
    for row in 0..h {
        put(canvas, x, y + row, color);
        put(canvas, x + w - 1, y + row, color);
    }
}

pub fn setup_palette() {
    set_palette_color(BG, 8, 30, 35);
    set_palette_color(TASKBAR, 6, 6, 8);
    set_palette_color(BODY_COLORS[0], 50, 50, 53);
    set_palette_color(BODY_COLORS[1], 45, 30, 50);
    set_palette_color(BODY_COLORS[2], 30, 45, 40);
    set_palette_color(BODY_COLORS[3], 50, 40, 25);
    set_palette_color(WINDOW_TITLE, 8, 12, 40);
    set_palette_color(WINDOW_BORDER, 63, 63, 63);
    set_palette_color(CURSOR, 63, 20, 20);
    set_palette_color(CLOSE_BTN, 55, 10, 10);
    set_palette_color(MINIMIZE_BTN, 55, 50, 10);
    set_palette_color(NEWWIN_TILE, 10, 45, 15);
    set_palette_color(CONSOLE_BG, 2, 2, 4);
}

pub fn init(fb: Framebuffer) {
    DESKTOP.lock().fb = Some(fb);
    render();
}

/// Pede pra redesenhar a tela agora -- usado por `gui_console` toda vez
/// que o texto do shell muda, ja que so' o mouse disparava `render()`
/// antes (Fase 22: teclado tambem precisa).
pub fn request_redraw() {
    render();
}

/// Retorna o retangulo do slot N da barra de tarefas (0 = botao "+", 1+ =
/// janelas minimizadas, na ordem em que aparecem em `windows`).
fn taskbar_slot_rect(slot: i32) -> (i32, i32, i32, i32) {
    let x = TASKBAR_TILE_GAP + slot * (TASKBAR_TILE_W + TASKBAR_TILE_GAP);
    let y = HEIGHT as i32 - TASKBAR_HEIGHT + (TASKBAR_HEIGHT - TASKBAR_TILE_H) / 2;
    (x, y, TASKBAR_TILE_W, TASKBAR_TILE_H)
}

fn render() {
    let mut d = DESKTOP.lock();

    for p in d.canvas.iter_mut() {
        *p = BG;
    }
    fill(&mut d.canvas, 0, HEIGHT as i32 - TASKBAR_HEIGHT, WIDTH as i32, TASKBAR_HEIGHT, TASKBAR);

    let (nx, ny, nw, nh) = taskbar_slot_rect(0);
    fill(&mut d.canvas, nx, ny, nw, nh, NEWWIN_TILE);
    {
        let canvas = &mut d.canvas;
        let mut putter = |px: i32, py: i32, color: u8| put(canvas, px, py, color);
        font8x8::draw_char(&mut putter, nx + 3, ny + 2, '+', TEXT_COLOR);
    }

    // Copia os dados das janelas pra fora antes de mexer em `d.canvas` --
    // iterar direto sobre `d.windows` manteria um emprestimo vivo que
    // colide com `&mut d.canvas` (os dois vem do mesmo `d` por tras do
    // MutexGuard).
    let minimized_colors: Vec<u8> = d
        .windows
        .iter()
        .filter(|w| w.minimized)
        .map(|w| w.body_color)
        .collect();
    for (i, color) in minimized_colors.into_iter().enumerate() {
        let (tx, ty, tw, th) = taskbar_slot_rect(1 + i as i32);
        fill(&mut d.canvas, tx, ty, tw, th, color);
    }

    #[allow(clippy::type_complexity)]
    let visible: Vec<(
        i32,
        i32,
        i32,
        i32,
        u8,
        String,
        bool,
        (i32, i32, i32, i32),
        (i32, i32, i32, i32),
    )> = d
        .windows
        .iter()
        .filter(|w| !w.minimized)
        .map(|w| {
            (
                w.x,
                w.y,
                w.w as i32,
                w.h as i32,
                w.body_color,
                w.label.clone(),
                w.is_terminal,
                w.minimize_button_rect(),
                w.close_button_rect(),
            )
        })
        .collect();
    for (x, y, w, h, color, label, is_terminal, (mx, my, mw, mh), (cx, cy, cw, ch)) in visible {
        fill(&mut d.canvas, x, y, w, h, color);
        fill(&mut d.canvas, x, y, w, TITLEBAR_HEIGHT, WINDOW_TITLE);
        rect(&mut d.canvas, x, y, w, h, WINDOW_BORDER);
        {
            let canvas = &mut d.canvas;
            let mut putter = |px: i32, py: i32, c: u8| put(canvas, px, py, c);
            font8x8::draw_text(&mut putter, x + 3, y + 2, &label, TEXT_COLOR);
        }
        if is_terminal {
            let (chars, cursor_col) = gui_console::snapshot();
            let content_x = x + TERMINAL_PAD;
            let content_y = y + TITLEBAR_HEIGHT + TERMINAL_PAD;
            let canvas = &mut d.canvas;
            let mut putter = |px: i32, py: i32, c: u8| put(canvas, px, py, c);
            for (row, line) in chars.iter().enumerate() {
                for (col, &byte) in line.iter().enumerate() {
                    if byte != b' ' {
                        font8x8::draw_char(
                            &mut putter,
                            content_x + col as i32 * font8x8::CHAR_WIDTH,
                            content_y + row as i32 * font8x8::CHAR_HEIGHT,
                            byte as char,
                            TEXT_COLOR,
                        );
                    }
                }
            }
            let cursor_x = content_x + cursor_col as i32 * font8x8::CHAR_WIDTH;
            let cursor_y = content_y + (gui_console::ROWS as i32 - 1) * font8x8::CHAR_HEIGHT;
            fill(canvas, cursor_x, cursor_y + font8x8::CHAR_HEIGHT - 1, font8x8::CHAR_WIDTH, 1, TEXT_COLOR);
        }
        fill(&mut d.canvas, mx, my, mw, mh, MINIMIZE_BTN);
        fill(&mut d.canvas, cx, cy, cw, ch, CLOSE_BTN);
    }

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

/// Clique (borda de subida do botao esquerdo) na barra de tarefas: slot 0
/// cria janela nova, slots seguintes restauram a N-esima janela minimizada
/// (na ordem em que aparecem em `windows`) e trazem ela pra frente.
fn handle_taskbar_click(d: &mut DesktopState, mx: i32, my: i32) {
    let (nx, ny, nw, nh) = taskbar_slot_rect(0);
    if point_in(mx, my, nx, ny, nw, nh) {
        d.spawn_window();
        return;
    }

    let mut slot = 1;
    let minimized_indices: Vec<usize> = d
        .windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.minimized)
        .map(|(i, _)| i)
        .collect();
    for idx in minimized_indices {
        let (tx, ty, tw, th) = taskbar_slot_rect(slot);
        if point_in(mx, my, tx, ty, tw, th) {
            let mut win = d.windows.remove(idx);
            win.minimized = false;
            d.windows.push(win);
            return;
        }
        slot += 1;
    }
}

/// Hit-test das janelas do topo pra base (`windows` em ordem de z-order,
/// ultimo = mais na frente). Fechar remove a janela de vez; minimizar so'
/// marca a flag; clicar em qualquer outro lugar da janela (corpo ou resto
/// da barra de titulo) traz ela pra frente e, se foi na barra de titulo,
/// comeca a arrastar.
fn handle_window_click(d: &mut DesktopState, mx: i32, my: i32) {
    let hit = d
        .windows
        .iter()
        .enumerate()
        .rev()
        .find(|(_, w)| !w.minimized && point_in(mx, my, w.x, w.y, w.w as i32, w.h as i32))
        .map(|(idx, _)| idx);

    let Some(idx) = hit else { return };

    let (cx, cy, cw, ch) = d.windows[idx].close_button_rect();
    if point_in(mx, my, cx, cy, cw, ch) {
        d.windows.remove(idx);
        return;
    }

    let (mnx, mny, mnw, mnh) = d.windows[idx].minimize_button_rect();
    if point_in(mx, my, mnx, mny, mnw, mnh) {
        let mut win = d.windows.remove(idx);
        win.minimized = true;
        d.windows.push(win);
        return;
    }

    let in_titlebar = my < d.windows[idx].y + TITLEBAR_HEIGHT;
    let win = d.windows.remove(idx);
    if in_titlebar {
        d.drag_offset = (mx - win.x, my - win.y);
        d.dragging = Some(d.windows.len()); // vai pro fim do Vec logo abaixo
    }
    d.windows.push(win);
}

pub fn handle_mouse(dx: i32, dy: i32, left: bool) {
    {
        let mut d = DESKTOP.lock();
        d.mouse_x = (d.mouse_x + dx).clamp(0, WIDTH as i32 - 1);
        d.mouse_y = (d.mouse_y - dy).clamp(0, HEIGHT as i32 - 1); // eixo Y do PS/2 e invertido

        let (mx, my) = (d.mouse_x, d.mouse_y);

        if left && !d.left_down {
            if my >= HEIGHT as i32 - TASKBAR_HEIGHT {
                handle_taskbar_click(&mut d, mx, my);
            } else {
                handle_window_click(&mut d, mx, my);
            }
        } else if !left {
            d.dragging = None;
        }
        d.left_down = left;

        if let Some(idx) = d.dragging {
            let (ox, oy) = d.drag_offset;
            if let Some(win) = d.windows.get_mut(idx) {
                win.x = mx - ox;
                win.y = my - oy;
            }
        }
    }

    render();
}
