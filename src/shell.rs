use crate::task::keyboard::ScancodeStream;
use crate::task::Task;
use crate::{fs, print, println, process, serial_println, task, vga_buffer};
use alloc::string::String;
use alloc::vec::Vec;
use futures_util::stream::StreamExt;
use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, KeyCode, ScancodeSet1};

const PROMPT: &str = "$ ";
const PROMPT_LEN: usize = 2;

struct Shell {
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    /// `None` = editando uma linha nova; `Some(i)` = navegando o historico
    /// (setas pra cima/baixo), `i` indexa `history`.
    history_index: Option<usize>,
}

impl Shell {
    fn new() -> Self {
        print!("{}", PROMPT);
        Shell {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
        }
    }

    fn redraw(&self) {
        vga_buffer::redraw_line(PROMPT_LEN, &self.buffer);
        vga_buffer::set_cursor(PROMPT_LEN + self.cursor);
    }

    fn handle_char(&mut self, c: char) {
        match c {
            '\n' => {
                println!();
                self.execute();
                self.buffer.clear();
                self.cursor = 0;
                self.history_index = None;
                print!("{}", PROMPT);
            }
            '\u{8}' => {
                if self.cursor > 0 {
                    self.buffer.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.history_index = None;
                    self.redraw();
                }
            }
            c if c.is_ascii_graphic() || c == ' ' => {
                self.buffer.insert(self.cursor, c);
                self.cursor += 1;
                self.history_index = None;
                self.redraw();
            }
            _ => {}
        }
    }

    fn handle_key_code(&mut self, key: KeyCode) {
        match key {
            KeyCode::ArrowLeft => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    vga_buffer::set_cursor(PROMPT_LEN + self.cursor);
                }
            }
            KeyCode::ArrowRight => {
                if self.cursor < self.buffer.len() {
                    self.cursor += 1;
                    vga_buffer::set_cursor(PROMPT_LEN + self.cursor);
                }
            }
            KeyCode::ArrowUp => self.history_navigate(true),
            KeyCode::ArrowDown => self.history_navigate(false),
            _ => {}
        }
    }

    /// Sobe (`older=true`) ou desce (`older=false`) no historico de
    /// comandos, substituindo a linha atual pelo comando encontrado.
    /// Descer alem do comando mais recente volta pra uma linha vazia.
    fn history_navigate(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }

        let next_index = match (self.history_index, older) {
            (None, true) => Some(self.history.len() - 1),
            (Some(i), true) => Some(i.saturating_sub(1)),
            (None, false) => None,
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };

        self.history_index = next_index;
        self.buffer = match next_index {
            Some(i) => self.history[i].clone(),
            None => String::new(),
        };
        self.cursor = self.buffer.len();
        self.redraw();
    }

    fn execute(&mut self) {
        let line = self.buffer.trim();
        if line.is_empty() {
            return;
        }

        if self.history.last().map(String::as_str) != Some(line) {
            self.history.push(String::from(line));
        }

        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        let cmd = String::from(cmd);

        match cmd.as_str() {
            "help" => println!(
                "comandos: help, echo, clear, ls, cat, write, rm, preempt, spawn, ps, kill, gui, net"
            ),
            "echo" => println!("{}", args.join(" ")),
            "clear" => vga_buffer::clear_screen(),
            "ls" => println!("{:?}", fs::FS.lock().list()),
            "cat" => match args.first() {
                Some(name) => match fs::FS.lock().read(name) {
                    Some(data) => match core::str::from_utf8(&data) {
                        Ok(text) => println!("{}", text),
                        Err(_) => println!("<arquivo binario>"),
                    },
                    None => println!("arquivo nao encontrado: {}", name),
                },
                None => println!("uso: cat <arquivo>"),
            },
            "write" => {
                if args.len() >= 2 {
                    let name = args[0];
                    let content = args[1..].join(" ");
                    fs::FS.lock().create(name, content.as_bytes());
                    println!("escrito: {}", name);
                } else {
                    println!("uso: write <arquivo> <conteudo>");
                }
            }
            "rm" => match args.first() {
                Some(name) => {
                    if fs::FS.lock().delete(name) {
                        println!("removido: {}", name);
                    } else {
                        println!("arquivo nao encontrado: {}", name);
                    }
                }
                None => println!("uso: rm <arquivo>"),
            },
            "preempt" => self.run_preempt_demo(),
            "spawn" => match args.first() {
                Some(name) => {
                    let name = String::from(*name);
                    self.run_spawn(&name);
                }
                None => println!("uso: spawn <arquivo>"),
            },
            "ps" => {
                let procs = process::list_background();
                if procs.is_empty() {
                    println!("nenhum processo em background");
                } else {
                    for (pid, name) in procs {
                        println!("{}\t{}", pid, name);
                    }
                }
            }
            "kill" => match args.first().and_then(|s| s.parse::<usize>().ok()) {
                Some(pid) => {
                    if process::kill_background(pid) {
                        println!("processo {} morto", pid);
                    } else {
                        println!("processo {} nao encontrado", pid);
                    }
                }
                None => println!("uso: kill <pid>"),
            },
            "gui" => {
                crate::draw_desktop();
                vga_buffer::enable_gui_mode();
            }
            "net" => self.run_net_init(),
            "dhcp" => match crate::net::dhcp::start() {
                Ok(()) => {
                    println!("dhcp: DISCOVER enviado, aguardando resposta...");
                    serial_println!("dhcp: DISCOVER enviado, aguardando resposta...");
                }
                Err(e) => println!("erro ao iniciar dhcp: {}", e),
            },
            _ => self.run_program(&cmd),
        }
    }

    /// Inicializa o driver de rede (Fase 23): acha o rtl8139 no PCI,
    /// reseta/configura ele e registra a task que fica escutando pacotes
    /// (ARP/ICMP) em background, mesma fila de spawn ja usada pra
    /// processos em background (`task::queue_spawn`, Fase 17).
    fn run_net_init(&mut self) {
        match crate::rtl8139::init() {
            Ok(mac) => {
                let ip = crate::net::our_ip();
                println!(
                    "rtl8139 inicializado: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, IP {}.{}.{}.{}",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                    ip[0], ip[1], ip[2], ip[3]
                );
                task::queue_spawn(Task::new(crate::net::NetPollTask));
                println!(
                    "escutando ARP/ICMP/UDP/TCP em background (echo UDP na porta {}, echo TCP na porta {})",
                    crate::net::UDP_ECHO_PORT, crate::net::TCP_ECHO_PORT
                );
            }
            Err(e) => println!("erro ao iniciar rede: {}", e),
        }
    }

    /// Roda um binario em background (Fase 17): diferente de `run_program`
    /// (que bloqueia o shell inteiro ate o processo sair), isso registra o
    /// processo e devolve o prompt na hora -- o processo roda em fatias
    /// curtas intercaladas com o shell via `process::BackgroundTask`.
    fn run_spawn(&mut self, name: &str) {
        let bytes = match fs::FS.lock().read(name) {
            Some(bytes) => bytes,
            None => {
                println!("arquivo nao encontrado: {}", name);
                return;
            }
        };

        let mut guard = process::KERNEL_MEMORY.lock();
        let (mapper, frame_allocator) = match guard.as_mut() {
            Some(mem) => mem,
            None => {
                println!("memoria do kernel indisponivel pra rodar processo");
                return;
            }
        };

        match process::spawn_background(mapper, frame_allocator, &bytes, String::from(name)) {
            Ok(pid) => {
                drop(guard);
                task::queue_spawn(Task::new(process::BackgroundTask::new(
                    pid,
                    String::from(name),
                )));
                println!("processo '{}' rodando em background (pid {})", name, pid);
            }
            Err(e) => println!("erro ao spawnar {}: {}", name, e),
        }
    }

    /// Demo isolada da Fase 14: roda looper_a e looper_b concorrentemente,
    /// trocados de verdade pelo timer (nao um de cada vez ate o fim, como
    /// o `run_program` normal). So volta pro prompt quando os dois saírem.
    fn run_preempt_demo(&mut self) {
        const LOOPER_A: &[u8] =
            include_bytes!("../programs/looper_a/target/x86_64-meu_os/release/looper_a");
        const LOOPER_B: &[u8] =
            include_bytes!("../programs/looper_b/target/x86_64-meu_os/release/looper_b");

        let mut guard = process::KERNEL_MEMORY.lock();
        let (mapper, frame_allocator) = match guard.as_mut() {
            Some(mem) => mem,
            None => {
                println!("memoria do kernel indisponivel pra rodar processo");
                return;
            }
        };

        println!("rodando looper_a + looper_b (preempcao real pelo timer)...");
        match process::run_preemptive(mapper, frame_allocator, &[LOOPER_A, LOOPER_B]) {
            Ok(()) => println!("demo de preempcao terminou (todos os processos saíram)"),
            Err(e) => println!("erro na demo de preempcao: {}", e),
        }
    }

    fn run_program(&mut self, cmd: &str) {
        let bytes = match fs::FS.lock().read(cmd) {
            Some(bytes) => bytes,
            None => {
                println!("comando desconhecido: {}", cmd);
                return;
            }
        };

        let mut guard = process::KERNEL_MEMORY.lock();
        let (mapper, frame_allocator) = match guard.as_mut() {
            Some(mem) => mem,
            None => {
                println!("memoria do kernel indisponivel pra rodar processo");
                return;
            }
        };

        match process::run_elf(mapper, frame_allocator, &bytes) {
            Ok(code) => println!("processo '{}' saiu com codigo {}", cmd, code),
            Err(e) => println!("erro ao executar {}: {}", cmd, e),
        }
    }
}

pub async fn run() {
    let mut shell = Shell::new();
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(layouts::Us104Key, ScancodeSet1, HandleControl::Ignore);

    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            match keyboard.process_keyevent(key_event) {
                Some(DecodedKey::Unicode(c)) => shell.handle_char(c),
                Some(DecodedKey::RawKey(key)) => shell.handle_key_code(key),
                None => {}
            }
        }
    }
}
